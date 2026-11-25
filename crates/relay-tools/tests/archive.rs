//! Archive against the test Redis (`IX_RELAY_TEST_URL`); skipped without one.

use relay_tools::archive::{Error, archive};
use std::os::unix::fs::PermissionsExt;

#[test]
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
