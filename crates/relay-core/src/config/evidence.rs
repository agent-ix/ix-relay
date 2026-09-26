//! Property evidence for configuration precedence (FR-026) and for the
//! configuration file as untrusted input (FR-026-AC-6, FR-027's loader half).
//!
//! Lives beside the loader so it can reach the private tier constants; the
//! process environment is shared, so every case takes [`ENV_LOCK`] and builds
//! its environment with [`EnvGuard`] (which clears every variable the loader
//! reads first, so nothing ambient leaks in). Each case writes its own file
//! under a per-process scratch directory and removes it.

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use proptest::prelude::*;

use super::*;
use crate::test_env::{ENV_LOCK, EnvGuard};

static CASE: AtomicU64 = AtomicU64::new(0);

/// A file path unique to this case, and removal of its directory on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir()
            .join(format!("ix-relay-config-evidence-{}", std::process::id()))
            .join(CASE.fetch_add(1, Ordering::Relaxed).to_string());
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        Self(dir.join("config.toml"))
    }

    fn write(&self, contents: &[u8]) {
        std::fs::write(&self.0, contents).expect("write config");
    }

    fn path(&self) -> &str {
        self.0.to_str().expect("utf-8 temp path")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if let Some(dir) = self.0.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// What each tier holds for one string key: a flag, an environment variable
/// (`Some("")` is set but empty) and a file value (never empty: an empty file
/// value is refused, see `tc_026_03_*`).
#[derive(Debug, Clone)]
struct Tiers {
    flag: Option<String>,
    env: Option<String>,
    file: Option<String>,
}

fn tiers(value: &'static str) -> impl Strategy<Value = Tiers> {
    let text = move || proptest::option::of(value);
    (
        text(),
        proptest::option::weighted(0.6, prop_oneof![Just(String::new()), value]),
        text(),
    )
        .prop_map(|(flag, env, file)| Tiers {
            flag: flag.filter(|s: &String| !s.is_empty()),
            env,
            file: file.filter(|s: &String| !s.is_empty()),
        })
}

/// The requirement's rule, as a function: flag, else non-empty environment,
/// else file, else default; and which tier that was.
fn expected(t: &Tiers, default: &str) -> (String, Source) {
    if let Some(v) = &t.flag {
        (v.clone(), Source::Flag)
    } else if let Some(v) = t.env.as_ref().filter(|v| !v.is_empty()) {
        (v.clone(), Source::Env)
    } else if let Some(v) = &t.file {
        (v.clone(), Source::File)
    } else {
        (default.to_owned(), Source::Default)
    }
}

/// The interval's tiers: `env` of `Some(None)` is set but empty.
#[derive(Debug, Clone)]
struct IntervalTiers {
    flag: Option<u64>,
    env: Option<Option<u64>>,
    file: Option<u64>,
}

fn interval_tiers() -> impl Strategy<Value = IntervalTiers> {
    (
        proptest::option::of(1u64..500),
        proptest::option::of(proptest::option::of(1u64..500)),
        proptest::option::of(1u64..500),
    )
        .prop_map(|(flag, env, file)| IntervalTiers { flag, env, file })
}

fn expected_interval(t: &IntervalTiers) -> (u64, Source) {
    if let Some(v) = t.flag {
        (v, Source::Flag)
    } else if let Some(v) = t.env.flatten() {
        (v, Source::Env)
    } else if let Some(v) = t.file {
        (v, Source::File)
    } else {
        (DEFAULT_WATCHER_INTERVAL_SECS, Source::Default)
    }
}

/// The file text holding exactly the present values.
fn file_text(
    url: &Tiers,
    machine: &Tiers,
    prefix: &Tiers,
    human: &Tiers,
    interval: Option<u64>,
) -> String {
    let mut text = String::new();
    for (key, t) in [
        ("url", url),
        ("machine", machine),
        ("prefix", prefix),
        ("human", human),
    ] {
        if let Some(v) = &t.file {
            text.push_str(&format!("{key} = \"{v}\"\n"));
        }
    }
    if let Some(secs) = interval {
        text.push_str(&format!("\n[watcher]\ninterval_secs = {secs}\n"));
    }
    text
}

fn cases(default: u32) -> ProptestConfig {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default);
    ProptestConfig {
        cases,
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

proptest! {
    #![proptest_config(cases(96))]

    /// Tracing: TC-026-01
    /// ACs: FR-026-AC-1
    ///
    /// For every key and every combination of flag, environment (absent, set
    /// or set-but-empty) and file value, the resolved value is the
    /// highest-precedence present one, else the default, and the reported
    /// source names that tier. Covers all five keys, including the watcher
    /// interval, and the machine name's normalisation.
    #[test]
    fn tc_026_01_the_highest_precedence_present_tier_wins_for_every_key(
        url in tiers("[a-z][a-z0-9:/.]{0,7}"),
        prefix in tiers("[a-z][a-z0-9:]{0,5}"),
        human in tiers("[a-z][a-z0-9-]{0,5}"),
        machine in tiers("[A-Za-z][A-Za-z0-9-]{0,6}(\\.[a-z]{1,4})?"),
        interval in interval_tiers(),
    ) {
        let _guard = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let scratch = Scratch::new();
        scratch.write(file_text(&url, &machine, &prefix, &human, interval.file).as_bytes());
        let interval_env = interval.env.map(|v| v.map_or_else(String::new, |n| n.to_string()));
        let mut env: Vec<(&'static str, &str)> = vec![(CONFIG_PATH_ENV, scratch.path())];
        for (name, t) in [(ENV_URL, &url), (ENV_PREFIX, &prefix), (ENV_HUMAN, &human), (ENV_MACHINE, &machine)] {
            if let Some(v) = &t.env {
                env.push((name, v));
            }
        }
        if let Some(v) = &interval_env {
            env.push((ENV_WATCHER_INTERVAL_SECS, v));
        }
        let _env = EnvGuard::isolated(&env);

        let flags = Flags {
            core: CoreFlags {
                url: url.flag.clone(),
                prefix: prefix.flag.clone(),
                machine: machine.flag.clone(),
                human: human.flag.clone(),
            },
            watcher_interval_secs: interval.flag,
        };
        let settings = load(flags.clone()).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let core = load_core(flags.core).map_err(|e| TestCaseError::fail(e.to_string()))?;

        for (name, got, t, default) in [
            ("url", &settings.core.url, &url, DEFAULT_URL),
            ("prefix", &settings.core.prefix, &prefix, crate::DEFAULT_PREFIX),
            ("human", &settings.core.human, &human, DEFAULT_HUMAN),
        ] {
            let (value, source) = expected(t, default);
            prop_assert_eq!(&got.value, &value, "{}", name);
            prop_assert_eq!(got.source, source, "{}", name);
        }
        let (machine_value, machine_source) = match (&machine.flag, machine.env.as_ref().filter(|v| !v.is_empty()), &machine.file) {
            (Some(v), _, _) => (v.clone(), Source::Flag),
            (None, Some(v), _) => (v.clone(), Source::Env),
            (None, None, Some(v)) => (v.clone(), Source::File),
            (None, None, None) => (crate::machine_name().unwrap().to_string(), Source::Default),
        };
        prop_assert_eq!(&settings.core.machine.value, &crate::MachineName::new(&machine_value).unwrap());
        prop_assert_eq!(settings.core.machine.source, machine_source);
        prop_assert_eq!(settings.watcher_interval_secs.value, expected_interval(&interval).0);
        prop_assert_eq!(settings.watcher_interval_secs.source, expected_interval(&interval).1);
        // The core loader (what `agent-msg` calls) agrees on the shared keys.
        prop_assert_eq!(&core.url.value, &settings.core.url.value);
        prop_assert_eq!(&core.machine.value, &settings.core.machine.value);
    }

    /// Tracing: TC-026-05
    /// ACs: FR-026-AC-5
    ///
    /// An interval variable that is set, non-empty and not an unsigned integer
    /// is refused by the watcher's loader whatever the file and flag hold
    /// (unless the flag outranks it), and never fails the core loader
    /// `agent-msg` uses.
    #[test]
    fn tc_026_05_a_bad_interval_variable_refuses_the_watcher_and_never_the_core_loader(
        raw in prop_oneof!["[a-z]{1,5}", "[0-9]{1,3}[a-z-]{1,2}", "-[0-9]{1,3}", "[0-9]{21,25}", "[0-9]{0,3}\\.[0-9]{1,2}"],
        file_secs in proptest::option::of(1u64..100),
        flag in proptest::option::of(1u64..100),
    ) {
        let _guard = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let scratch = Scratch::new();
        scratch.write(file_secs.map_or_else(String::new, |n| format!("[watcher]\ninterval_secs = {n}\n")).as_bytes());
        let _env = EnvGuard::isolated(&[(CONFIG_PATH_ENV, scratch.path()), (ENV_WATCHER_INTERVAL_SECS, &raw)]);
        let is_number = raw.bytes().all(|b| b.is_ascii_digit()) && raw.parse::<u64>().is_ok();
        prop_assume!(!is_number);
        let got = load(Flags { core: CoreFlags::default(), watcher_interval_secs: flag });
        // Refused, not skipped in favour of the file or the default. (The flag
        // tier is resolved after the environment is parsed, so it does not rescue it.)
        prop_assert!(
            matches!(got, Err(ConfigError::BadEnvValue { name: ENV_WATCHER_INTERVAL_SECS, .. })),
            "{raw:?} -> {got:?}"
        );
        prop_assert!(load_core(CoreFlags::default()).is_ok());
    }

    /// Tracing: TC-026-03, TC-026-06
    /// ACs: FR-026-AC-3, FR-026-AC-6
    ///
    /// A file with an empty `url`, `machine`, `prefix` or `human`, or with any
    /// key the schema does not know (including `name`, which is not a file
    /// key), fails to load as malformed and names both the file and, for an
    /// empty value, the key.
    #[test]
    fn tc_026_03_an_empty_value_or_an_unknown_key_is_malformed_and_names_the_file(
        key in proptest::sample::select(vec!["url", "machine", "prefix", "human"]),
        unknown in "[a-z_]{1,8}",
    ) {
        prop_assume!(!["url", "machine", "prefix", "human", "watcher"].contains(&unknown.as_str()));
        let _guard = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let scratch = Scratch::new();
        let _env = EnvGuard::isolated(&[(CONFIG_PATH_ENV, scratch.path())]);

        scratch.write(format!("{key} = \"\"\n").as_bytes());
        let empty = load_file();
        let Err(ConfigError::Malformed { path, message, .. }) = &empty else {
            return Err(TestCaseError::fail(format!("empty {key}: {empty:?}")));
        };
        prop_assert_eq!(path.as_str(), scratch.path());
        prop_assert!(message.contains(key), "{message:?} does not name {key}");

        scratch.write(format!("{unknown} = \"x\"\n").as_bytes());
        let unknown_key = load_file();
        prop_assert!(
            matches!(&unknown_key, Err(ConfigError::Malformed { path, .. }) if path == scratch.path()),
            "unknown key {unknown}: {unknown_key:?}"
        );
    }

    /// Tracing: TC-026-04
    /// ACs: FR-026-AC-4
    ///
    /// A machine name that normalises to the empty string is refused, whichever
    /// tier supplies it.
    #[test]
    fn tc_026_04_a_machine_that_normalises_to_empty_is_refused_from_every_tier(
        raw in prop_oneof![Just(".".to_owned()), "\\.[a-z]{0,5}"], tier in 0usize..3,
    ) {
        let _guard = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let scratch = Scratch::new();
        let mut flags = CoreFlags::default();
        let mut env: Vec<(&'static str, &str)> = vec![(CONFIG_PATH_ENV, scratch.path())];
        match tier {
            0 => flags.machine = Some(raw.clone()),
            1 => env.push((ENV_MACHINE, &raw)),
            _ => scratch.write(format!("machine = \"{raw}\"\n").as_bytes()),
        }
        let _env = EnvGuard::isolated(&env);
        let got = load_core(flags);
        prop_assert!(matches!(got, Err(ConfigError::Malformed { .. })), "{raw:?} via tier {tier}: {got:?}");
    }
}

/// Tracing: TC-026-02
/// ACs: FR-026-AC-2
///
/// A set-but-empty variable is unset, for every string key: the file's value
/// is used, not an empty string and not the default.
#[test]
fn tc_026_02_an_empty_variable_falls_through_to_the_file_for_every_key() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let scratch = Scratch::new();
    scratch.write(b"url = \"redis://file:1\"\nmachine = \"file-machine\"\nprefix = \"file:\"\nhuman = \"file-human\"\n\n[watcher]\ninterval_secs = 9\n");
    let _env = EnvGuard::isolated(&[
        (CONFIG_PATH_ENV, scratch.path()),
        (ENV_URL, ""),
        (ENV_MACHINE, ""),
        (ENV_PREFIX, ""),
        (ENV_HUMAN, ""),
        (ENV_WATCHER_INTERVAL_SECS, ""),
    ]);
    let s = load(Flags::default()).expect("load");
    assert_eq!(
        (
            s.core.url.value.as_str(),
            s.core.machine.value.as_str(),
            s.core.prefix.value.as_str(),
            s.core.human.value.as_str(),
            s.watcher_interval_secs.value
        ),
        ("redis://file:1", "file-machine", "file:", "file-human", 9)
    );
    for source in [
        s.core.url.source,
        s.core.machine.source,
        s.core.prefix.source,
        s.core.human.source,
        s.watcher_interval_secs.source,
    ] {
        assert_eq!(source, Source::File);
    }
}

/// Tracing: TC-026-02
/// ACs: FR-026-AC-2
///
/// Found by `tc_026_01_*` for `IX_RELAY_MACHINE`, checked for every key: with no
/// file, a set-but-empty variable falls through to the default (for the
/// machine, the hostname) rather than being refused as an empty name (the default tier read the variable without the empty-is-unset
/// rule every other tier applies).
#[test]
fn tc_026_02_an_empty_variable_with_no_file_falls_through_to_the_default_for_every_key() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let scratch = Scratch::new();
    let _env = EnvGuard::isolated(&[
        (CONFIG_PATH_ENV, scratch.path()),
        (ENV_URL, ""),
        (ENV_MACHINE, ""),
        (ENV_PREFIX, ""),
        (ENV_HUMAN, ""),
        (ENV_WATCHER_INTERVAL_SECS, ""),
    ]);
    let s = load(Flags::default()).expect("an empty variable is unset");
    assert_eq!(s.core.url.value, DEFAULT_URL);
    assert_eq!(s.core.prefix.value, crate::DEFAULT_PREFIX);
    assert_eq!(s.core.human.value, DEFAULT_HUMAN);
    assert_eq!(s.watcher_interval_secs.value, DEFAULT_WATCHER_INTERVAL_SECS);
    assert_eq!(
        s.core.machine.value,
        crate::MachineName::new(&gethostname::gethostname().to_string_lossy()).unwrap()
    );
    for source in [
        s.core.url.source,
        s.core.machine.source,
        s.core.prefix.source,
        s.core.human.source,
        s.watcher_interval_secs.source,
    ] {
        assert_eq!(source, Source::Default);
    }
}

// ---------------------------------------------------------------------------
// The file as untrusted input: seed corpus and fuzz lane (FR-026-AC-6)
// ---------------------------------------------------------------------------

/// The contract of `load_file` over any file content, whatever it is: it
/// returns rather than panics, a failure names the file, a syntax failure has a
/// 1-based position inside the text, the message is one line, an unreadable
/// file is exactly one that is not UTF-8, and a file that loads holds no empty
/// string value and only known keys.
fn check_load_file_contract(bytes: &[u8]) -> Result<(), TestCaseError> {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let scratch = Scratch::new();
    scratch.write(bytes);
    let _env = EnvGuard::isolated(&[(CONFIG_PATH_ENV, scratch.path())]);
    let is_utf8 = std::str::from_utf8(bytes);
    match load_file() {
        Ok(Loaded::Present(cfg)) => {
            prop_assert!(is_utf8.is_ok(), "a non-UTF-8 file loaded");
            for v in [&cfg.url, &cfg.machine, &cfg.prefix, &cfg.human] {
                prop_assert!(v.as_deref() != Some(""), "an empty value was accepted");
            }
        }
        Ok(Loaded::Absent) => prop_assert!(false, "the file exists but reported absent"),
        Err(ConfigError::Unreadable { path, .. }) => {
            prop_assert!(is_utf8.is_err(), "a UTF-8 file was unreadable");
            prop_assert_eq!(path.as_str(), scratch.path());
        }
        Err(ConfigError::Malformed {
            path,
            line,
            column,
            message,
        }) => {
            prop_assert!(is_utf8.is_ok(), "a non-UTF-8 file was parsed");
            prop_assert_eq!(path.as_str(), scratch.path());
            prop_assert!(line >= 1 && column >= 1, "position {line}:{column}");
            let lines = is_utf8.map_or(0, |t| t.split('\n').count());
            prop_assert!(
                line <= lines.max(1),
                "line {line} past the {lines} lines of the file"
            );
            prop_assert!(!message.contains('\n'), "multi-line message {message:?}");
        }
        Err(other) => prop_assert!(false, "unexpected error kind: {other:?}"),
    }
    Ok(())
}

/// Tracing: TC-026-06
/// ACs: FR-026-AC-6
///
/// Seed corpus, always run: known-nasty file contents (the criterion's own
/// `name = "x"`, wrong types, duplicate keys, BOM, CRLF, NULs, deep tables, a
/// megabyte line, invalid UTF-8) all satisfy the loader's contract.
#[test]
fn tc_026_06_the_seed_corpus_of_nasty_files_satisfies_the_loader_contract() {
    let long_line = format!("url = \"{}\"\n", "x".repeat(1 << 20));
    let deep = "[a]\n".repeat(2000);
    let corpus: Vec<&[u8]> = vec![
        b"name = \"x\"\n",
        b"",
        b"\n\n\n",
        b"url = 5\n",
        b"url = \"a\"\nurl = \"b\"\n",
        b"\xef\xbb\xbfurl = \"redis://x\"\n",
        b"url = \"a\"\r\nprefix = \"b\"\r\n",
        b"url = \"unterminated\n",
        b"url = \n",
        b"\0\0\0",
        b"\xff\xfe\x00",
        b"[watcher]\ninterval_secs = -1\n",
        b"[watcher]\ninterval_secs = 99999999999999999999\n",
        b"[watcher]\ninterval_secs = 2\nextra = 1\n",
        b"url.host = \"x\"\n",
        b"watcher = 5\n",
        b"machine = \".\"\n",
        long_line.as_bytes(),
        deep.as_bytes(),
    ];
    for bytes in corpus {
        check_load_file_contract(bytes).unwrap_or_else(|e| {
            panic!(
                "{:?}: {e}",
                String::from_utf8_lossy(&bytes[..bytes.len().min(40)])
            )
        });
    }
    // The criterion itself: `name` is not a file key.
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let scratch = Scratch::new();
    scratch.write(b"name = \"x\"\n");
    let _env = EnvGuard::isolated(&[(CONFIG_PATH_ENV, scratch.path())]);
    assert!(matches!(load_file(), Err(ConfigError::Malformed { .. })));
}

proptest! {
    #![proptest_config(cases(64))]

    /// Tracing: TC-026-06
    /// ACs: FR-026-AC-6
    #[test]
    fn tc_026_06_any_file_content_satisfies_the_loader_contract(
        bytes in proptest::collection::vec(any::<u8>(), 0..200),
    ) {
        check_load_file_contract(&bytes)?;
    }
}

/// TOML-shaped text: known and unknown keys, values of every type, tables,
/// comments, and the separators the parser is picky about.
fn toml_like() -> impl Strategy<Value = String> {
    let key = prop_oneof![
        proptest::sample::select(vec![
            "url",
            "machine",
            "prefix",
            "human",
            "name",
            "watcher",
            "interval_secs",
            "a.b",
            "\"q k\""
        ])
        .prop_map(str::to_owned),
        "[a-z_]{1,6}",
    ];
    let value = prop_oneof![
        "\"[ -~]{0,8}\"",
        "-?[0-9]{1,22}",
        Just("true".to_owned()),
        Just("[]".to_owned()),
        Just("{ a = 1 }".to_owned()),
        Just("\"\"".to_owned()),
        Just("'lit'".to_owned()),
    ];
    let line = prop_oneof![
        4 => (key.clone(), value).prop_map(|(k, v)| format!("{k} = {v}")),
        1 => key.prop_map(|k| format!("[{k}]")),
        1 => Just("# comment".to_owned()),
        1 => Just(String::new()),
        1 => "[ -~]{0,12}",
    ];
    proptest::collection::vec(line, 0..10).prop_map(|lines| lines.join("\n"))
}

proptest! {
    #![proptest_config(cases(64))]

    /// Tracing: TC-026-06
    /// ACs: FR-026-AC-6
    #[test]
    fn tc_026_06_toml_shaped_files_satisfy_the_loader_contract(text in toml_like()) {
        check_load_file_contract(text.as_bytes())?;
    }
}

proptest! {
    #![proptest_config(cases(20_000))]

    /// Tracing: TC-026-06
    /// ACs: FR-026-AC-6
    ///
    /// Fuzz lane: arbitrary bytes and TOML-shaped text through the loader, at
    /// a raised case count. Every result satisfies [`check_load_file_contract`].
    #[test]
    #[ignore = "fuzz lane: run with `make fuzz` (cargo test -- --ignored fuzz_)"]
    fn fuzz_config_file_over_arbitrary_bytes(bytes in proptest::collection::vec(any::<u8>(), 0..1024)) {
        check_load_file_contract(&bytes)?;
    }

    /// Tracing: TC-026-06
    /// ACs: FR-026-AC-6
    #[test]
    #[ignore = "fuzz lane: run with `make fuzz` (cargo test -- --ignored fuzz_)"]
    fn fuzz_config_file_over_toml_shaped_text(text in toml_like()) {
        check_load_file_contract(text.as_bytes())?;
    }
}
