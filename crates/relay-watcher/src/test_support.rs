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

/// The command names (upper-cased) in one chunk of client-to-server bytes.
fn command_names(chunk: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(chunk);
    let mut names = Vec::new();
    for command in text.split("\r\n*").enumerate().map(|(i, part)| {
        if i == 0 {
            part.strip_prefix('*').unwrap_or(part)
        } else {
            part
        }
    }) {
        // `<n>\r\n$<len>\r\n<NAME>\r\n...`
        let mut lines = command.split("\r\n");
        if let (Some(_count), Some(_len), Some(name)) = (lines.next(), lines.next(), lines.next()) {
            names.push(name.to_ascii_uppercase());
        }
    }
    names
}

/// A TCP proxy in front of a real Redis that injects faults per command, so a
/// test can make Redis vanish at an exact point in a tick. The proxy thread
/// lives until the test process exits.
pub(crate) struct FaultProxy {
    /// `redis://` URL to connect through the proxy.
    pub(crate) url: String,
    /// Every command name that reached the proxy, in order, faulted or not.
    pub(crate) seen: Arc<Mutex<Vec<String>>>,
    /// The same names grouped by request: a pipeline is one entry, so the entry
    /// count is the number of round trips.
    pub(crate) requests: Arc<Mutex<Vec<Vec<String>>>>,
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
        let requests = Arc::new(Mutex::new(Vec::new()));
        let requests_by_thread = Arc::clone(&requests);
        thread::spawn(move || {
            for client in listener.incoming().flatten() {
                let Ok(server) = TcpStream::connect(&upstream) else {
                    continue;
                };
                let (policy, client_read) = (Arc::clone(&policy), client.try_clone().unwrap());
                let seen = Arc::clone(&seen_by_thread);
                let requests = Arc::clone(&requests_by_thread);
                let (server_read, mut client_write) =
                    (server.try_clone().unwrap(), client.try_clone().unwrap());
                // Redis to client: straight through.
                thread::spawn(move || {
                    let mut server_read = server_read;
                    let mut buf = [0u8; 4096];
                    while let Ok(n) = server_read.read(&mut buf) {
                        if n == 0 || client_write.write_all(&buf[..n]).is_err() {
                            break;
                        }
                    }
                });
                // Client to Redis: consult the policy per command.
                thread::spawn(move || {
                    let (mut client_read, mut server) = (client_read, server);
                    let mut buf = [0u8; 4096];
                    let mut silent = false;
                    while let Ok(n) = client_read.read(&mut buf) {
                        if n == 0 {
                            break;
                        }
                        let names = command_names(&buf[..n]);
                        seen.lock().unwrap().extend(names.iter().cloned());
                        requests.lock().unwrap().push(names.clone());
                        if silent {
                            continue;
                        }
                        let fault = names
                            .iter()
                            .map(|name| (policy.lock().unwrap())(name))
                            .find(|fault| *fault != Fault::Forward)
                            .unwrap_or(Fault::Forward);
                        match fault {
                            Fault::Forward => {
                                if server.write_all(&buf[..n]).is_err() {
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
        Self {
            url,
            seen,
            requests,
        }
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

    #[test]
    fn command_names_reads_single_and_pipelined_commands() {
        assert_eq!(command_names(b"*1\r\n$7\r\nHGETALL\r\n"), ["HGETALL"]);
        assert_eq!(
            command_names(
                b"*4\r\n$6\r\nCLIENT\r\n$7\r\nSETINFO\r\n$8\r\nLIB-NAME\r\n$2\r\nab\r\n\
                  *2\r\n$4\r\nllen\r\n$1\r\nk\r\n"
            ),
            ["CLIENT", "LLEN"]
        );
    }
}
