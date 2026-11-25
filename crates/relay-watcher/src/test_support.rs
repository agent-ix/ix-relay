//! Test-only helpers shared by the watcher's unit tests.

use std::{
    io::{Read, Write},
    net::TcpStream,
    sync::{Arc, Mutex},
    thread,
};

/// A connection-level error, as a real dead Redis produces it: a refused
/// connect. Built directly rather than by connecting to a freed port: a
/// bind-drop-connect probe races every other test that binds port 0, and a
/// lucky one accepts the connection.
pub(crate) fn connection_error() -> relay_core::Error {
    relay_core::Error::Redis(std::io::Error::from(std::io::ErrorKind::ConnectionRefused).into())
}

/// A call a signal interrupted (`EINTR`), as a stop by SIGTERM produces in the blocked
/// wait on the message log.
pub(crate) fn interrupted_error() -> relay_core::Error {
    relay_core::Error::Redis(std::io::Error::from(std::io::ErrorKind::Interrupted).into())
}

/// An error that leaves the connection usable.
pub(crate) fn data_error() -> relay_core::Error {
    relay_core::Error::InvalidTicket { ticket: "x".into() }
}

/// What a [`FaultProxy`] does with a client command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fault {
    /// Pass it to Redis.
    Forward,
    /// Drop it and close the connection, as a reset tunnel does.
    Cut,
    /// Drop it and every later byte and leave the connection open, as a
    /// blackholed tunnel does: the client's read times out.
    Silent,
}

fn read_command(parser: &mut redis::Parser, reader: impl Read) -> Option<Vec<Vec<u8>>> {
    redis::from_redis_value(parser.parse_value(reader).ok()?).ok()
}

/// Command names grouped by actual client calls, independent of TCP framing and replies.
pub(crate) fn request_groups(log: &relay_core::test_support::RequestLog) -> Vec<Vec<String>> {
    log.snapshot()
        .into_iter()
        .map(|packed| {
            let mut parser = redis::Parser::new();
            let mut reader = std::io::Cursor::new(packed);
            let mut names = Vec::new();
            while let Some(command) = read_command(&mut parser, &mut reader) {
                names.push(
                    // A script by SHA or by body is one script call.
                    String::from_utf8(command[0].clone())
                        .unwrap()
                        .to_ascii_uppercase()
                        .replace("EVALSHA", "EVAL"),
                );
            }
            names
        })
        .collect()
}

/// A TCP proxy in front of a real Redis that injects faults per command, so a
/// test can make Redis vanish at an exact point in a tick. The proxy thread
/// lives until the test process exits.
pub(crate) struct FaultProxy {
    /// `redis://` URL to connect through the proxy.
    pub(crate) url: String,
    /// Every command name that reached the proxy, in order, faulted or not.
    pub(crate) seen: Arc<Mutex<Vec<String>>>,
}

impl FaultProxy {
    /// Proxies to the Redis at `upstream_url`; `policy` sees each command name
    /// in order (across every connection) and decides its fate.
    pub(crate) fn start(
        upstream_url: &str,
        policy: impl FnMut(&str) -> Fault + Send + 'static,
    ) -> Self {
        let upstream = upstream_url.trim_start_matches("redis://").to_owned();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("redis://{}", listener.local_addr().unwrap());
        let policy = Arc::new(Mutex::new(policy));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_by_thread = Arc::clone(&seen);
        thread::spawn(move || {
            for client in listener.incoming().flatten() {
                let Ok(server) = TcpStream::connect(&upstream) else {
                    continue;
                };
                let (policy, client_read) = (Arc::clone(&policy), client.try_clone().unwrap());
                let seen = Arc::clone(&seen_by_thread);
                let (server_read, mut client_write) =
                    (server.try_clone().unwrap(), client.try_clone().unwrap());
                // Redis to client: straight through.
                thread::spawn(move || {
                    let mut server_read = server_read;
                    let mut buf = [0u8; 4096];
                    while let Ok(n) = server_read.read(&mut buf) {
                        if n == 0 {
                            break;
                        }
                        if client_write.write_all(&buf[..n]).is_err() {
                            break;
                        }
                    }
                });
                // Client to Redis: consult the policy per command.
                thread::spawn(move || {
                    let mut client_read = client_read;
                    let mut server = server;
                    let mut parser = redis::Parser::new();
                    let mut silent = false;
                    while let Some(command) = read_command(&mut parser, &mut client_read) {
                        let Some(name) = command.first() else { break };
                        let name = String::from_utf8_lossy(name).to_ascii_uppercase();
                        seen.lock().unwrap().push(name.clone());
                        if silent {
                            continue;
                        }
                        let fault = (policy.lock().unwrap())(&name);
                        match fault {
                            Fault::Forward => {
                                if server.write_all(&redis::pack_command(&command)).is_err() {
                                    break;
                                }
                            }
                            Fault::Cut => {
                                let _ = client_read.shutdown(std::net::Shutdown::Both);
                                let _ = server.shutdown(std::net::Shutdown::Both);
                                break;
                            }
                            Fault::Silent => silent = true,
                        }
                    }
                });
            }
        });
        Self { url, seen }
    }
}

/// Sends one raw command to the Redis at `url` and returns its first reply
/// line, so a test can set up a key of the wrong type without a Redis client.
pub(crate) fn raw_command(url: &str, args: &[&str]) -> String {
    let mut stream = TcpStream::connect(url.trim_start_matches("redis://")).unwrap();
    let mut request = format!("*{}\r\n", args.len());
    for arg in args {
        request.push_str(&format!("${}\r\n{arg}\r\n", arg.len()));
    }
    stream.write_all(request.as_bytes()).unwrap();
    let mut buf = [0u8; 256];
    let n = stream.read(&mut buf).unwrap();
    String::from_utf8_lossy(&buf[..n]).trim_end().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ix_trace_rs::trace;
    use redis::{Commands, IntoConnectionInfo};
    use relay_test_support::Bus;
    use std::{io::Cursor, time::Duration};

    fn connection(proxy: &FaultProxy) -> redis::Connection {
        let info = proxy.url.as_str().into_connection_info().unwrap();
        let settings = info.redis_settings().clone().set_skip_set_lib_name();
        redis::Client::open(info.set_redis_settings(settings))
            .unwrap()
            .get_connection()
            .unwrap()
    }

    #[test]
    #[trace("FR-038-AC-5", "FR-045-AC-22")]
    fn fragmented_script_and_coalesced_commands_keep_their_command_boundaries() {
        struct Fragments(Cursor<Vec<u8>>);
        impl Read for Fragments {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                let length = buffer.len().min(4096);
                self.0.read(&mut buffer[..length])
            }
        }
        let script = format!(
            "local x = [=[{}]=]; return 1",
            "\r\n*1\r\n$4\r\nSCAN\r\n".repeat(1024)
        );
        let pipeline = redis::pipe()
            .cmd("PING")
            .cmd("EVAL")
            .arg(&script)
            .arg(0)
            .cmd("LLEN")
            .arg("a")
            .cmd("LLEN")
            .arg("b")
            .get_packed_pipeline();
        assert!(pipeline.len() > 4096);
        let mut reader = Fragments(Cursor::new(pipeline));
        let mut parser = redis::Parser::new();
        assert_eq!(
            read_command(&mut parser, &mut reader).unwrap(),
            [b"PING".to_vec()]
        );
        let first = read_command(&mut parser, &mut reader).unwrap();
        assert_eq!(first[0], b"EVAL");
        assert_eq!(first[1], script.as_bytes());
        for key in [b"a", b"b"] {
            let command = read_command(&mut parser, &mut reader).unwrap();
            assert_eq!(command, [b"LLEN".to_vec(), key.to_vec()]);
        }
    }

    #[test]
    #[trace("FR-038-AC-5", "FR-045-AC-22")]
    fn proxy_forwards_complete_large_and_coalesced_commands() {
        let Some(bus) = Bus::connect("watcher-proxy-frames") else {
            return;
        };
        let proxy = FaultProxy::start(&bus.url, |_| Fault::Forward);
        let mut connection = connection(&proxy);
        let script = format!(
            "local x = [=[{}]=]; return 1",
            "\r\n*1\r\n$4\r\nSCAN\r\n".repeat(1024)
        );
        let result: Vec<usize> = redis::pipe()
            .cmd("EVAL")
            .arg(script)
            .arg(0)
            .cmd("LLEN")
            .arg(bus.key("a"))
            .cmd("LLEN")
            .arg(bus.key("b"))
            .query(&mut connection)
            .unwrap();
        assert_eq!(result, [1, 0, 0]);
        let pong: String = redis::cmd("PING").query(&mut connection).unwrap();
        assert_eq!(pong, "PONG");
        assert_eq!(
            *proxy.seen.lock().unwrap(),
            ["EVAL", "LLEN", "LLEN", "PING"]
        );
    }

    #[test]
    fn proxy_faults_apply_once_to_complete_commands_with_arbitrary_bulk_arguments() {
        let Some(mut bus) = Bus::connect("watcher-proxy-faults") else {
            return;
        };
        for fault in [Fault::Forward, Fault::Cut, Fault::Silent] {
            let key = bus.key(&format!("{fault:?}"));
            let proxy =
                FaultProxy::start(
                    &bus.url,
                    move |name| if name == "SET" { fault } else { Fault::Forward },
                );
            let mut connection = connection(&proxy);
            connection
                .set_read_timeout(Some(Duration::from_millis(100)))
                .unwrap();
            let value = "\r\n*1\r\n$4\r\nSCAN\r\n".repeat(1024);
            let result: redis::RedisResult<String> = redis::cmd("SET")
                .arg(&key)
                .arg(&value)
                .query(&mut connection);
            let stored: Option<String> = bus.raw.get(&key).unwrap();
            if fault == Fault::Forward {
                assert_eq!(result.unwrap(), "OK");
                assert_eq!(stored.as_deref(), Some(value.as_str()));
            } else {
                assert!(result.is_err());
                assert_eq!(stored, None);
            }
            assert_eq!(*proxy.seen.lock().unwrap(), ["SET"]);
        }
    }
}
