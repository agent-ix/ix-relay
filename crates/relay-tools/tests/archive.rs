//! Archive against the test Redis (`IX_RELAY_TEST_URL`); skipped without one.

use ix_trace_rs::trace;
use relay_tools::archive::{Error, archive, archive_within};
use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};

#[test]
#[trace("FR-073-AC-2")]
fn binary_dump_ttl_source_preservation_and_exclusive_output() {
    let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
        eprintln!("skipping: IX_RELAY_TEST_URL unset");
        return;
    };
    let mut conn = redis::Client::open(url.as_str())
        .unwrap()
        .get_connection()
        .unwrap();
    let prefix = format!("test:archive:{}:", std::process::id());
    let mut key = prefix.clone().into_bytes();
    key.push(0xff);
    let value: &[u8] = b"\x00\r\n\xff";
    let () = redis::cmd("SET")
        .arg(&key)
        .arg(value)
        .arg("PX")
        .arg(60000)
        .query(&mut conn)
        .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("archive.jsonl");
    let archived = archive(&url, &prefix, &output);
    let stored: Vec<u8> = redis::cmd("GET").arg(&key).query(&mut conn).unwrap();
    let second = archive(&url, &prefix, &output);
    let () = redis::cmd("DEL").arg(&key).query(&mut conn).unwrap();

    assert_eq!(archived.unwrap(), 1);
    let rows: Vec<serde_json::Value> = std::fs::read_to_string(&output)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(rows[0]["prefix"], prefix.as_str());
    assert!(rows[1]["ttl_ms"].as_i64().unwrap() > 0);
    assert_eq!(stored, value);
    assert_eq!(
        std::fs::metadata(&output).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(matches!(second, Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::AlreadyExists));
}

#[test]
#[trace("FR-073-AC-2")]
fn refuses_the_new_namespace_and_a_non_redis_url_before_connecting() {
    let out = std::path::Path::new("/nonexistent/never-created");
    assert!(matches!(
        archive("redis://127.0.0.1:1", "ixr:v3:", out),
        Err(Error::Prefix)
    ));
    assert!(matches!(
        archive("redis://127.0.0.1:1", "", out),
        Err(Error::Prefix)
    ));
    assert!(matches!(archive("http://x", "old:", out), Err(Error::Url)));
}

#[test]
#[trace("FR-073-AC-2")]
fn rediss_is_refused_with_a_message_naming_why() {
    let out = std::path::Path::new("/nonexistent/never-created");
    let err = archive("rediss://127.0.0.1:1", "old:", out).unwrap_err();
    assert!(matches!(err, Error::Tls));
    assert!(err.to_string().contains("no TLS library"));
}

#[test]
#[trace("FR-073-AC-2")]
fn a_server_that_never_replies_fails_within_the_timeout() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("redis://{}", listener.local_addr().unwrap());
    let holder = std::thread::spawn(move || {
        let held = listener.accept();
        std::thread::sleep(Duration::from_secs(3));
        drop(held);
    });
    let dir = tempfile::tempdir().unwrap();
    let started = Instant::now();
    let result = archive_within(
        &url,
        "old:",
        &dir.path().join("a.jsonl"),
        Duration::from_secs(1),
    );
    assert!(matches!(result, Err(Error::Redis(_))), "{result:?}");
    let elapsed = started.elapsed();
    // One deadline in all: a connect and a first read that each took the
    // full timeout would measure about two seconds here.
    assert!(elapsed < Duration::from_millis(1500), "{elapsed:?}");
    holder.join().unwrap();
}
