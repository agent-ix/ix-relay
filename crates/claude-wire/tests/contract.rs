//! Contract tests: every fixture under `fixtures/claude-2.1.283/` is parsed through the
//! crate and the exact typed result is asserted. A change on Claude Code's side that
//! these fixtures capture cannot pass unnoticed; a new Claude Code shape is added as a
//! new fixture and a new row here.

use std::{collections::BTreeSet, fs, path::PathBuf};

use claude_wire::{
    CAPTURED_ON, ControlLine, DriftReason, Envelope, Ignored, Receipt, RecordVerdict,
    SessionRecord, TranscriptMatch, Verdict, parse_control_line, scan, wrap,
};

const SESSION: &str = "6b1f0c1e-9d5a-4f3a-8a11-2c7e4d9a0b55";

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(format!("claude-{CAPTURED_ON}"))
}

fn read(name: &str) -> Vec<u8> {
    match fs::read(dir().join(name)) {
        Ok(bytes) => bytes,
        Err(e) => panic!("fixture {name}: {e}"),
    }
}

fn text(name: &str) -> String {
    match String::from_utf8(read(name)) {
        Ok(text) => text,
        Err(e) => panic!("fixture {name} is not UTF-8: {e}"),
    }
}

/// What a fixture must parse to.
enum Expect {
    Control(ControlLine),
    Record(RecordVerdict),
    RecordUndecodable,
    Transcript(TranscriptMatch),
    /// The outgoing line, rebuilt from its parts, equals the fixture byte for byte.
    Envelope,
    /// The wrapper, rebuilt, equals the fixture byte for byte.
    Wrapper,
}

fn receipt(orig: Option<&str>, dropped: &[&str], verdict: Verdict) -> ControlLine {
    ControlLine::Receipt(Receipt {
        orig_msg_id: orig.map(str::to_owned),
        dropped_msg_ids: dropped.iter().map(|s| (*s).to_owned()).collect(),
        verdict,
    })
}

fn drift(reason: DriftReason) -> ControlLine {
    ControlLine::Unrecognised { reason }
}

fn expectations() -> Vec<(&'static str, Expect)> {
    use Expect::{Control, Record, RecordUndecodable, Transcript};
    let m = Some("cb4ae227");
    vec![
        (
            "receipt-refused.json",
            Control(receipt(m, &[], Verdict::Refused)),
        ),
        ("receipt-held.json", Control(receipt(m, &[], Verdict::Held))),
        (
            "receipt-delivered.json",
            Control(receipt(m, &[], Verdict::Delivered)),
        ),
        (
            "receipt-denied.json",
            Control(receipt(m, &[], Verdict::Denied)),
        ),
        (
            "receipt-hold-timeout.json",
            Control(receipt(m, &[], Verdict::HoldExpired)),
        ),
        (
            "receipt-dropped.json",
            Control(receipt(None, &["cb4ae227", "d4e5f607"], Verdict::Dropped)),
        ),
        (
            "receipt-duplicate.json",
            Control(receipt(m, &[], Verdict::Duplicate)),
        ),
        (
            "liveness-blank.txt",
            Control(ControlLine::KnownIgnored(Ignored::Liveness)),
        ),
        ("session-record.json", Record(RecordVerdict::Supported)),
        (
            "transcript-consumed.jsonl",
            Transcript(TranscriptMatch::Acked),
        ),
        (
            "transcript-not-yet.jsonl",
            Transcript(TranscriptMatch::Absent),
        ),
        ("envelope-line.json", Expect::Envelope),
        ("wrapper.txt", Expect::Wrapper),
        (
            "drift-receipt-unknown-status.json",
            Control(receipt(m, &[], Verdict::Unknown)),
        ),
        (
            "drift-control-action-renamed.json",
            Control(drift(DriftReason::ActionUnknown)),
        ),
        (
            "drift-control-status-missing.json",
            Control(drift(DriftReason::StatusMissing)),
        ),
        (
            "drift-control-type-renamed.json",
            Control(drift(DriftReason::TypeMissing)),
        ),
        ("drift-not-json.txt", Control(ControlLine::NotJson)),
        (
            "drift-session-record-protocol-2.json",
            Record(RecordVerdict::UnsupportedProtocol {
                peer_protocol: Some(2),
                version: Some("2.1.283".into()),
            }),
        ),
        (
            "drift-session-record-old-version.json",
            Record(RecordVerdict::UnsupportedVersion {
                version: Some("2.1.100".into()),
                peer_protocol: Some(1),
            }),
        ),
        (
            "drift-session-record-no-socket.json",
            Record(RecordVerdict::Malformed {
                missing: "messagingSocketPath",
            }),
        ),
        ("drift-session-record-no-session-id.json", RecordUndecodable),
        (
            "drift-transcript-origin-renamed.jsonl",
            Transcript(TranscriptMatch::ShapeUnrecognised),
        ),
        (
            "drift-transcript-origin-string.jsonl",
            Transcript(TranscriptMatch::ShapeUnrecognised),
        ),
    ]
}

/// The provenance each fixture must carry, pinned per file: relabelling one fails the test.
fn provenance() -> Vec<(&'static str, &'static str)> {
    const CAPTURED: &str = "captured";
    const SHAPE: &str = "captured-shape";
    const DERIVED: &str = "derived-from-program-text";
    const SYNTHETIC: &str = "synthetic";
    vec![
        ("receipt-refused.json", SHAPE),
        ("receipt-held.json", SHAPE),
        ("receipt-delivered.json", DERIVED),
        ("receipt-denied.json", DERIVED),
        ("receipt-hold-timeout.json", DERIVED),
        ("receipt-dropped.json", DERIVED),
        ("receipt-duplicate.json", DERIVED),
        ("liveness-blank.txt", CAPTURED),
        ("session-record.json", SHAPE),
        ("transcript-consumed.jsonl", SHAPE),
        ("transcript-not-yet.jsonl", SHAPE),
        ("envelope-line.json", SHAPE),
        ("wrapper.txt", SHAPE),
        ("drift-receipt-unknown-status.json", SYNTHETIC),
        ("drift-control-action-renamed.json", SYNTHETIC),
        ("drift-control-status-missing.json", SYNTHETIC),
        ("drift-control-type-renamed.json", SYNTHETIC),
        ("drift-not-json.txt", SYNTHETIC),
        ("drift-session-record-protocol-2.json", SYNTHETIC),
        ("drift-session-record-old-version.json", SYNTHETIC),
        ("drift-session-record-no-socket.json", SYNTHETIC),
        ("drift-session-record-no-session-id.json", SYNTHETIC),
        ("drift-transcript-origin-renamed.jsonl", SYNTHETIC),
        ("drift-transcript-origin-string.jsonl", SYNTHETIC),
    ]
}

/// Tracing: TC-046-08
/// ACs: FR-046-AC-8
///
/// Every fixture file is in the manifest with exactly its pinned provenance, every manifest
/// row has a file, and every file has an expectation here.
#[test]
fn the_manifest_the_directory_and_the_expectations_agree() {
    let manifest = text("MANIFEST");
    let mut listed = std::collections::BTreeMap::new();
    for line in manifest
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
    {
        let cols: Vec<&str> = line.split('\t').collect();
        assert_eq!(cols.len(), 3, "file, provenance, note: {line}");
        assert!(
            listed
                .insert(cols[0].to_owned(), cols[1].to_owned())
                .is_none(),
            "{} listed twice",
            cols[0]
        );
    }
    let pinned: std::collections::BTreeMap<String, String> = provenance()
        .into_iter()
        .map(|(n, p)| (n.to_owned(), p.to_owned()))
        .collect();
    assert_eq!(listed, pinned, "manifest provenance vs the pinned table");
    let on_disk: BTreeSet<String> = fs::read_dir(dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n != "MANIFEST")
        .collect();
    assert_eq!(
        listed.keys().cloned().collect::<BTreeSet<_>>(),
        on_disk,
        "manifest vs directory"
    );
    let expected: BTreeSet<String> = expectations()
        .iter()
        .map(|(n, _)| (*n).to_owned())
        .collect();
    assert_eq!(expected, on_disk, "expectations vs directory");
    // Only unedited live bytes are `captured`.
    let captured: Vec<&String> = listed
        .iter()
        .filter(|(_, p)| *p == "captured")
        .map(|(n, _)| n)
        .collect();
    assert_eq!(captured, ["liveness-blank.txt"]);
}

/// The exact typed result of every fixture.
#[test]
fn every_fixture_parses_to_its_exact_typed_result() {
    for (name, expect) in expectations() {
        match expect {
            Expect::Control(expected) => {
                assert_eq!(parse_control_line(&text(name)), expected, "{name}");
            }
            Expect::Record(expected) => {
                let record = SessionRecord::parse(&read(name)).unwrap();
                assert_eq!(record.verdict(), expected, "{name}");
            }
            Expect::RecordUndecodable => {
                assert_eq!(SessionRecord::parse(&read(name)), None, "{name}");
            }
            Expect::Transcript(expected) => {
                assert_eq!(scan(&read(name), SESSION), expected, "{name}");
            }
            Expect::Envelope => {
                let content = wrap(
                    "uds:/run/user/1000/cc-socks/7.sock",
                    "hub:EA:1",
                    "[ix-relay] hi",
                );
                let line = Envelope::new(
                    SESSION,
                    "sess-1",
                    "uds:/run/user/1000/cc-socks/7.sock",
                    &content,
                )
                .line()
                .unwrap();
                assert_eq!(line, text(name), "{name}");
            }
            Expect::Wrapper => {
                assert_eq!(
                    wrap(
                        "uds:/run/user/1000/cc-socks/7.sock",
                        "hub:EA:1",
                        "[ix-relay] hi"
                    ),
                    text(name),
                    "{name}"
                );
            }
        }
    }
}

/// Tracing: TC-046-08
/// ACs: FR-046-AC-8
///
/// The captured session record exposes the fields the watcher reads, exactly.
#[test]
fn the_captured_session_record_decodes_its_fields() {
    let r = SessionRecord::parse(&read("session-record.json")).unwrap();
    assert_eq!(r.pid, 4242);
    assert_eq!(r.session_id, SESSION);
    assert_eq!(r.proc_start.as_deref(), Some("1000"));
    assert_eq!(r.kind.as_deref(), Some("interactive"));
    assert_eq!(r.version.as_deref(), Some(CAPTURED_ON));
    assert_eq!(r.peer_protocol, Some(1));
    assert_eq!(
        r.messaging_socket_path.as_deref().and_then(|p| p.to_str()),
        Some("/run/user/1000/cc-socks/4242.sock")
    );
}

/// Tracing: TC-046-01
/// ACs: FR-046-AC-1
///
/// The crate boundary: only serde and serde_json in, no relay crate, no async runtime,
/// no I/O crates.
#[test]
fn the_crate_depends_on_serde_and_serde_json_only() {
    let manifest =
        fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();
    let mut section = "";
    let mut deps = BTreeSet::new();
    for line in manifest.lines() {
        let line = line.trim();
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = if name == "dependencies" {
                "deps"
            } else {
                "other"
            };
        } else if section == "deps" && !line.is_empty() {
            let name = line.split(['=', '.', ' ']).next().unwrap();
            deps.insert(name.to_owned());
        }
    }
    let expected: BTreeSet<String> = ["serde", "serde_json"]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    assert_eq!(deps, expected);
}

/// Tracing: TC-046-14
/// ACs: FR-046-AC-14
///
/// No I/O in the library sources: no identifier of a file, socket, process, environment or
/// thread API appears in the code (comments and the test module excluded), whatever the
/// import form (`use std::{fs, io}` included), and `std` is reached only for paths.
#[test]
fn the_library_does_no_io() {
    const BANNED: [&str; 16] = [
        "fs",
        "io",
        "net",
        "process",
        "env",
        "thread",
        "File",
        "OpenOptions",
        "TcpStream",
        "UdpSocket",
        "UnixStream",
        "UnixListener",
        "Command",
        "Stdin",
        "Stdout",
        "read_to_string",
    ];
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    for entry in fs::read_dir(src).unwrap() {
        let path = entry.unwrap().path();
        let body = fs::read_to_string(&path).unwrap();
        let library = body.split("#[cfg(test)]").next().unwrap();
        let code: String = library
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .map(|l| l.split("//").next().unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !code.contains("std::{"),
            "{}: a grouped std import hides its members",
            path.display()
        );
        for token in code.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
            assert!(!BANNED.contains(&token), "{}: {token}", path.display());
        }
    }
}
