//! Machine-level config file: `~/.ix/relay/config.toml` (AGE-2017).
//!
//! Every other agent-ix tool keeps its settings under `~/.ix/<tool>/`; this is
//! that file for `ix-relay`. One loader lives here so `agent-msg` and
//! `relay-watcher` read exactly the same file the exact same way, instead of
//! each growing its own.
//!
//! Precedence is **flag > env > file > default**, resolved once per key by
//! [`ix_cli_kit::config::resolve`] rather than a hand-rolled chain. A missing
//! file means defaults ([`ix_cli_kit::config::Loaded::Absent`]); a malformed
//! one is a [`ConfigError`] naming the path, line and column.
//!
//! Two loaders, because only `relay-watcher` cares about `[watcher]`:
//! [`load_core`] resolves `url`, `machine`, `prefix` and `human` and is what
//! `agent-msg` calls — a bad `IX_RELAY_WATCHER_INTERVAL_SECS` can never fail
//! it, because it never reads that key. [`load`] resolves everything,
//! including `[watcher] interval_secs`, for `relay-watcher`.
//!
//! A set-but-empty env var is treated as unset (falls through to the next
//! tier); an empty `url`, `machine`, `prefix` or `human` *in the file* is
//! refused outright, and so is a `machine` that normalizes
//! ([`crate::MachineName`]) to an empty string regardless of which
//! tier supplied it.

use std::path::{Path, PathBuf};

use ix_cli_kit::config::{self, ParseLocation};
use serde::Deserialize;

pub mod otlp;

/// A configuration failure: an unreadable or malformed file, or an
/// environment variable that doesn't parse.
pub use ix_cli_kit::config::ConfigError;
/// Whether a config file was found at all, distinct from a parsed empty one.
pub use ix_cli_kit::config::Loaded;
/// A resolved value together with which precedence tier supplied it.
pub use ix_cli_kit::config::Resolved;
/// Which precedence tier supplied a [`Resolved`] value.
pub use ix_cli_kit::config::Source;

/// Overrides the config file path. Meant for tests, so they never touch a
/// real `~/.ix/relay/config.toml`.
pub const CONFIG_PATH_ENV: &str = "IX_RELAY_CONFIG";

/// Redis URL used when nothing else supplies one.
pub const DEFAULT_URL: &str = "redis://127.0.0.1:6379";

/// The human inbox `escalate`/`resolve` notify/sign as, used when nothing
/// else supplies one. One name because today there's one human on the bus;
/// the `human` key (and `IX_RELAY_HUMAN`/`--human`) exist so a rename or a
/// second inbox never needs a code change.
pub const DEFAULT_HUMAN: &str = "operator";

/// How soon `relay-watcher` looks again at a lead that has mail queued but was
/// left for its own hook, in seconds, used when nothing else supplies one. New
/// messages wake the watcher at once whatever this is.
pub const DEFAULT_WATCHER_INTERVAL_SECS: u64 = 2;

const ENV_URL: &str = "IX_RELAY_URL";
const ENV_MACHINE: &str = "IX_RELAY_MACHINE";
const ENV_PREFIX: &str = "IX_RELAY_PREFIX";
const ENV_HUMAN: &str = "IX_RELAY_HUMAN";
const ENV_WATCHER_INTERVAL_SECS: &str = "IX_RELAY_WATCHER_INTERVAL_SECS";

/// How `relay-watcher` gets a popped batch into a lead's session (FR-044).
///
/// Off by default: an unset `[watcher] delivery` is [`DeliveryMode::Herdr`], so
/// nothing changes for anyone who has not opted in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeliveryMode {
    /// Type the batch in through `herdr agent prompt` (the default).
    #[default]
    Herdr,
    /// Post the batch to the lead's Claude Code inbox socket, and fall back to
    /// herdr for a batch the socket certainly did not take.
    Socket,
}

impl DeliveryMode {
    /// The value as it is written in `config.toml`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Herdr => "herdr",
            Self::Socket => "socket",
        }
    }
}

/// The `[watcher]` table in `config.toml`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WatcherFile {
    /// How often `relay-watcher` polls, in seconds.
    pub interval_secs: Option<u64>,
    /// The OTLP collector the watcher pushes metrics to (FR-045); absent means
    /// nothing is sent. A raw value, on purpose: the shared file type must load
    /// whatever these watcher-only keys hold, so a typo never stops `agent-msg` or
    /// a hook. Only `relay-watcher` validates them ([`load`]), and `agent-msg
    /// doctor` reports them ([`otlp::file_problem`]).
    pub otlp_endpoint: Option<toml::Value>,
    /// Request headers for the collector, `name=value,name=value`; raw like
    /// `otlp_endpoint`.
    pub otlp_headers: Option<toml::Value>,
    /// Seconds between pushes (at least 5); raw like `otlp_endpoint`.
    pub otlp_interval_secs: Option<toml::Value>,
    /// How batches reach a lead: `"herdr"` (default) or `"socket"`. A raw value, tolerated
    /// like the OTLP keys: only `relay-watcher` validates it ([`load`]), so a typo never
    /// stops `agent-msg` or a hook, and `agent-msg doctor` reports it
    /// ([`watcher_delivery`]).
    pub delivery: Option<toml::Value>,
}

/// The shape of `config.toml`. Every field is optional: an absent key falls
/// through to the next layer down (env, then the compiled-in default), same
/// as an absent file does. A key that *is* present but an empty string
/// (`url`, `machine` or `prefix`) is refused — see [`load_file`].
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileConfig {
    /// Redis URL of the bus.
    pub url: Option<String>,
    /// This machine's name on the bus, before [`crate::MachineName`].
    pub machine: Option<String>,
    /// Redis key prefix.
    pub prefix: Option<String>,
    /// The human inbox `escalate`/`resolve` notify/sign as.
    pub human: Option<String>,
    /// Watcher settings.
    #[serde(default)]
    pub watcher: WatcherFile,
}

/// Values an invoking CLI got on its own command line, to feed the flag tier
/// of the precedence chain for the three keys every ix-relay binary shares.
/// `None` means either the binary offers no flag for that key, or the user
/// didn't pass it — both fall through to env.
#[derive(Debug, Clone, Default)]
pub struct CoreFlags {
    /// `--url`.
    pub url: Option<String>,
    /// `--prefix`.
    pub prefix: Option<String>,
    /// This machine's name. No CLI currently exposes a flag for it; kept so
    /// the chain is complete and testable like every other key.
    pub machine: Option<String>,
    /// `--human` (`agent-msg`'s global flag).
    pub human: Option<String>,
}

/// [`CoreFlags`] plus `relay-watcher`'s own `--interval-secs`. Only
/// `relay-watcher` builds one of these; `agent-msg` builds a bare
/// [`CoreFlags`] and calls [`load_core`] instead.
#[derive(Debug, Clone, Default)]
pub struct Flags {
    /// The three keys every binary resolves.
    pub core: CoreFlags,
    /// `relay-watcher`'s `--interval-secs`.
    pub watcher_interval_secs: Option<u64>,
}

/// The bus settings every ix-relay binary resolves: connection and identity.
#[derive(Debug, Clone)]
pub struct CoreSettings {
    /// Redis URL.
    pub url: Resolved<String>,
    /// Redis key prefix.
    pub prefix: Resolved<String>,
    /// This machine's name, already normalized (see
    /// [`crate::MachineName`]) and guaranteed non-empty.
    pub machine: Resolved<crate::MachineName>,
    /// The human inbox `escalate`/`resolve` notify/sign as.
    pub human: Resolved<String>,
}

/// [`CoreSettings`] plus `relay-watcher`'s `[watcher]` keys.
#[derive(Debug, Clone)]
pub struct Settings {
    /// The three keys every binary resolves.
    pub core: CoreSettings,
    /// `relay-watcher`'s poll interval, in seconds.
    pub watcher_interval_secs: Resolved<u64>,
    /// Where the watcher pushes OTLP metrics, if anywhere (FR-045).
    pub watcher_otlp: otlp::OtlpSettings,
    /// `[watcher] delivery`: [`DeliveryMode::Herdr`] unless the file opts in.
    pub delivery: DeliveryMode,
}

/// Where `config.toml` lives: `$IX_RELAY_CONFIG` when set, else
/// `$HOME/.ix/relay/config.toml`. `None` when neither is available — treated
/// the same as a missing file, not an error.
#[must_use]
pub fn config_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(CONFIG_PATH_ENV) {
        return Some(PathBuf::from(path));
    }
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join(".ix")
            .join("relay")
            .join("config.toml"),
    )
}

/// Where machine-local runtime state files live: a `state` directory beside
/// [`config_path`]'s file, so `$IX_RELAY_CONFIG` redirects it too (tests and
/// alternate installs) and the default is `$HOME/.ix/relay/state`. `None` when
/// [`config_path`] is.
#[must_use]
pub fn state_dir() -> Option<PathBuf> {
    Some(config_path()?.parent()?.join("state"))
}

/// The directory [`config_path`] lives in: where `ix-relay`'s other local
/// state (the hook failure spool, in `spool/`) is kept, so anything that
/// redirects the config file with `IX_RELAY_CONFIG` redirects that state too.
/// `None` under the same conditions as [`config_path`].
#[must_use]
pub fn config_dir() -> Option<PathBuf> {
    config_path()?.parent().map(Path::to_path_buf)
}

/// Loads `config.toml` from [`config_path`].
///
/// # Errors
///
/// [`ConfigError::Unreadable`] if the file exists and can't be read;
/// [`ConfigError::Malformed`] if it exists and doesn't parse as TOML into
/// [`FileConfig`] — including an unrecognized key (refused by
/// `#[serde(deny_unknown_fields)]`) or an empty `url`, `machine` or `prefix`
/// (refused as an empty required value). Either way the error names the
/// file; a TOML syntax error also names the line and column.
pub fn load_file() -> Result<Loaded<FileConfig>, ConfigError> {
    let Some(path) = config_path() else {
        return Ok(Loaded::Absent);
    };
    let loaded = config::load_with(&path, |text| {
        toml::from_str::<FileConfig>(text).map_err(|error| toml_error_parts(text, &error))
    })?;
    if let Loaded::Present(cfg) = &loaded {
        validate_file_config(cfg, &path)?;
    }
    Ok(loaded)
}

/// Resolves [`CoreSettings`]: loads `config.toml` (a missing file is
/// defaults) and applies flag > env > file > default to `url`, `machine` and
/// `prefix`. Never touches `IX_RELAY_WATCHER_INTERVAL_SECS` or
/// `[watcher]` — a bad or malformed watcher setting can't fail this.
///
/// # Errors
///
/// As [`load_file`], plus a `machine` that normalizes to an empty string
/// (see [`crate::MachineName`]) regardless of which tier supplied
/// the raw value.
pub fn load_core(flags: CoreFlags) -> Result<CoreSettings, ConfigError> {
    let file = load_file()?.unwrap_or(FileConfig::default());
    resolve_core(flags, &file)
}

/// Resolves [`Settings`]: [`load_core`]'s three keys, plus
/// `[watcher] interval_secs`, for `relay-watcher`.
///
/// # Errors
///
/// As [`load_core`], plus [`ConfigError::BadEnvValue`] when
/// `IX_RELAY_WATCHER_INTERVAL_SECS` is set to a non-empty value that doesn't
/// parse as a number: an unparsable env value is refused, never silently
/// skipped in favor of the file or the default.
pub fn load(flags: Flags) -> Result<Settings, ConfigError> {
    let file = load_file()?.unwrap_or(FileConfig::default());
    let core = resolve_core(flags.core, &file)?;
    let watcher_interval_secs = resolve_watcher_interval(flags.watcher_interval_secs, &file)?;
    let watcher_otlp = otlp::resolve(&file)?;
    Ok(Settings {
        core,
        watcher_interval_secs,
        watcher_otlp,
        delivery: delivery_from_file(&file)?,
    })
}

fn resolve_core(flags: CoreFlags, file: &FileConfig) -> Result<CoreSettings, ConfigError> {
    let url = config::resolve(
        flags.url,
        env_string(ENV_URL),
        file.url.clone(),
        DEFAULT_URL.to_owned(),
    );
    let prefix = config::resolve(
        flags.prefix,
        env_string(ENV_PREFIX),
        file.prefix.clone(),
        crate::DEFAULT_PREFIX.to_owned(),
    );
    // `crate::machine_name()` is itself env-or-hostname, normalized: reused
    // here as the default tier so this chain has exactly one place that knows
    // how to fall back to the hostname. The final value is normalized again
    // below regardless of which tier won — normalization is idempotent, and a
    // value from the flag, env or file tier hasn't been through it yet.
    let raw_machine = config::resolve(
        flags.machine,
        env_string(ENV_MACHINE),
        file.machine.clone(),
        // A default that cannot be a machine name becomes "", which the
        // refusal below reports like any other unusable value.
        crate::machine_name().map(String::from).unwrap_or_default(),
    );
    let Ok(normalized) = crate::MachineName::new(&raw_machine.value) else {
        // Whatever supplied this (a flag, an env var, or the file — the file
        // case is already caught earlier by validate_file_config, but e.g. a
        // raw value of "." normalizes to "" from any tier) leaves the bus
        // with no usable machine name. Refuse rather than register leads
        // under "".
        return Err(empty_value_error(config_path().as_deref(), "machine"));
    };
    let machine = Resolved {
        value: normalized,
        source: raw_machine.source,
        origin: raw_machine.origin,
    };
    let human = config::resolve(
        flags.human,
        env_string(ENV_HUMAN),
        file.human.clone(),
        DEFAULT_HUMAN.to_owned(),
    );
    Ok(CoreSettings {
        url,
        prefix,
        machine,
        human,
    })
}

fn resolve_watcher_interval(
    flag: Option<u64>,
    file: &FileConfig,
) -> Result<Resolved<u64>, ConfigError> {
    // A set-but-empty env var is unset, same as every other key here — but
    // `resolve_parsed` would otherwise treat "" as a value that fails to
    // parse as u64 and refuse it as a BadEnvValue, which is the wrong error
    // for "nothing was there."
    let is_empty = std::env::var_os(ENV_WATCHER_INTERVAL_SECS).is_some_and(|v| v.is_empty());
    let env = if is_empty {
        None
    } else {
        config::resolve_parsed::<u64>(ENV_WATCHER_INTERVAL_SECS)?
    }
    .map(|value| (ENV_WATCHER_INTERVAL_SECS, value));
    Ok(config::resolve(
        flag,
        env,
        file.watcher.interval_secs,
        DEFAULT_WATCHER_INTERVAL_SECS,
    ))
}

/// `name`, treating a set-but-empty value as unset like every other environment
/// key here.
fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

/// The one key [`delivery_from_file`] reads strictly.
#[derive(Deserialize)]
struct StrictDeliveryFile {
    #[serde(default)]
    watcher: StrictDelivery,
}

#[derive(Deserialize, Default)]
struct StrictDelivery {
    delivery: Option<DeliveryMode>,
}

/// The file's `[watcher] delivery`, validated: [`DeliveryMode::Herdr`] when absent.
/// Only reads the file again when it holds the key, so the typed read reports a
/// typo at its real line and column. Only the watcher and `agent-msg doctor` call
/// this: every other binary loads the raw value and ignores it.
fn delivery_from_file(file: &FileConfig) -> Result<DeliveryMode, ConfigError> {
    if file.watcher.delivery.is_none() {
        return Ok(DeliveryMode::Herdr);
    }
    let Some(path) = config_path() else {
        return Ok(DeliveryMode::Herdr);
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(DeliveryMode::Herdr);
    };
    match toml::from_str::<StrictDeliveryFile>(&text) {
        Ok(strict) => Ok(strict.watcher.delivery.unwrap_or_default()),
        Err(error) => {
            let (location, message) = toml_error_parts(&text, &error);
            Err(ConfigError::Malformed {
                path: path.display().to_string(),
                line: location.line,
                column: location.column,
                message: format!("delivery: {message}"),
            })
        }
    }
}

/// The delivery mode this machine's file asks for, or why the watcher would refuse
/// it: what `agent-msg doctor` reads. The file only.
///
/// # Errors
///
/// The reason the file does not load or its `delivery` value is not `"herdr"` or
/// `"socket"`.
pub fn watcher_delivery() -> Result<DeliveryMode, String> {
    let file = load_file()
        .map_err(|e| e.to_string())?
        .unwrap_or(FileConfig::default());
    delivery_from_file(&file).map_err(|e| e.to_string())
}

/// Reads `name`, treating a set-but-empty value the same as unset: an
/// operator's `export IX_RELAY_URL=` left over from unsetting it shouldn't
/// silently win over the config file with an empty string.
fn env_string(name: &'static str) -> Option<(&'static str, String)> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => Some((name, value)),
        _ => None,
    }
}

/// Refuses an empty `url`, `machine` or `prefix` *in the file* — distinct
/// from [`env_string`]'s "empty env means unset", because a key the operator
/// deliberately wrote into the file as `""` is far more likely a mistake
/// (a botched template, a stripped-out value) than a considered choice, and
/// there is no "next tier" within the file itself to fall through to.
fn validate_file_config(cfg: &FileConfig, path: &Path) -> Result<(), ConfigError> {
    for (key, value) in [
        ("url", cfg.url.as_deref()),
        ("machine", cfg.machine.as_deref()),
        ("prefix", cfg.prefix.as_deref()),
        ("human", cfg.human.as_deref()),
    ] {
        if value == Some("") {
            return Err(empty_value_error(Some(path), key));
        }
    }
    Ok(())
}

/// Builds the [`ConfigError::Malformed`] for an empty `key`. Reuses that
/// variant rather than a bespoke one: ix-cli-kit's `ConfigError` is closed
/// (this crate can't add a variant to it), and `Malformed` is already "this
/// resolved configuration is not usable," which an empty required value is.
/// `path` is `None` when the empty value didn't come from the file (e.g. a
/// `machine` that normalizes to empty from a flag or env value).
fn empty_value_error(path: Option<&Path>, key: &str) -> ConfigError {
    let path = path.map_or_else(
        || "<no config file>".to_owned(),
        |p| p.display().to_string(),
    );
    ConfigError::Malformed {
        path,
        line: 1,
        column: 1,
        message: format!("{key} must not be empty"),
    }
}

/// The location and a single-line message for a TOML parse failure.
///
/// `toml::de::Error`'s own `Display` renders a multi-line box (a source
/// excerpt with a `^` pointer beneath it); reusing that as `ConfigError::
/// Malformed`'s `message` would make `{path}:{line}:{column}: {message}`
/// repeat the position twice and sprawl across several lines. `.message()`
/// is the error's own short description with none of that formatting.
fn toml_error_parts(text: &str, error: &toml::de::Error) -> (ParseLocation, String) {
    let loc = error.span().map_or(ParseLocation::default(), |span| {
        let (line, column) = line_col(text, span.start);
        ParseLocation { line, column }
    });
    (loc, error.message().to_owned())
}

/// 1-based (line, column) of the character at `byte_offset` in `text`.
/// Counts characters rather than slicing, so an offset that isn't a char
/// boundary (shouldn't happen for a toml span, but isn't this function's
/// invariant to assume) can't panic.
fn line_col(text: &str, byte_offset: usize) -> (usize, usize) {
    let mut line = 1usize;
    let mut column = 1usize;
    for (index, ch) in text.char_indices() {
        if index >= byte_offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    (line, column)
}

/// `url` with everything between `://` and the *last* `@` replaced by `***`,
/// for logging.
///
/// A Redis URL is `redis://user:pass@host:port`; the password must never reach
/// a log or a terminal. An unencoded `/`, `?` or `#` inside the password would
/// end the authority early if it were parsed properly, so this deliberately
/// does not: it treats the last `@` as the end of the userinfo, which can only
/// over-redact (a `@` in a query string hides the host too), never leak. Text
/// with no `://` or no `@` after it is returned unchanged.
#[must_use]
pub fn redact_url(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_owned();
    };
    match rest.rsplit_once('@') {
        Some((_userinfo, after)) => format!("{scheme}://***@{after}"),
        None => url.to_owned(),
    }
}

#[cfg(test)]
mod evidence;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_env::{ENV_LOCK, EnvGuard};

    fn write_config(dir: &std::path::Path, contents: &str) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, contents).unwrap();
        path
    }

    fn scratch_dir(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join("ix-relay-config-test")
            .join(format!("{name}-{}", std::process::id()))
    }

    #[test]
    fn missing_file_means_defaults() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = scratch_dir("missing");
        std::fs::remove_dir_all(&dir).ok();
        let _env =
            EnvGuard::isolated(&[(CONFIG_PATH_ENV, dir.join("config.toml").to_str().unwrap())]);

        let loaded = load_file().unwrap();
        assert_eq!(loaded, Loaded::Absent);

        let settings = load(Flags::default()).unwrap();
        assert_eq!(settings.core.url.value, DEFAULT_URL);
        assert_eq!(settings.core.url.source, Source::Default);
        assert_eq!(settings.core.prefix.value, crate::DEFAULT_PREFIX);
        assert_eq!(settings.core.human.value, DEFAULT_HUMAN);
        assert_eq!(settings.core.human.source, Source::Default);
        assert_eq!(
            settings.watcher_interval_secs.value,
            DEFAULT_WATCHER_INTERVAL_SECS
        );
    }

    #[test]
    fn malformed_file_names_path_line_and_column_on_a_single_line() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = scratch_dir("malformed");
        // An unterminated string on line 2: TOML basic strings can't span a
        // newline, so this is an unambiguous syntax error at a known spot.
        let path = write_config(&dir, "url = \"redis://x\"\nmachine = \"unterminated\n");
        let _env = EnvGuard::isolated(&[(CONFIG_PATH_ENV, path.to_str().unwrap())]);

        let error = load_file().unwrap_err();
        let ConfigError::Malformed {
            path: err_path,
            line,
            column,
            ..
        } = &error
        else {
            panic!("expected Malformed, got {error}");
        };
        assert_eq!(err_path, path.to_str().unwrap());
        assert_eq!(*line, 2);
        assert_eq!(*column, 24);
        let rendered = error.to_string();
        assert!(
            rendered.starts_with(&format!("{}:2:24:", path.display())),
            "{rendered}"
        );
        assert!(
            !rendered.contains('\n'),
            "expected a single-line message: {rendered}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unknown_key_is_refused() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = scratch_dir("unknown-key");
        let path = write_config(&dir, "url = \"redis://x\"\nbogus = true\n");
        let _env = EnvGuard::isolated(&[(CONFIG_PATH_ENV, path.to_str().unwrap())]);

        let error = load_file().unwrap_err();
        assert!(matches!(error, ConfigError::Malformed { .. }), "{error}");
        let rendered = error.to_string();
        assert!(rendered.contains("bogus"), "{rendered}");
        assert!(!rendered.contains('\n'), "{rendered}");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Tracing: TC-044-01
    /// ACs: FR-044-AC-1
    #[test]
    fn tc_044_01_the_mode_is_herdr_unless_the_file_says_socket() {
        let _guard = ENV_LOCK.lock().unwrap();
        let mode = |toml: &str| {
            let dir = scratch_dir("delivery-mode");
            let path = write_config(&dir, toml);
            let _env = EnvGuard::isolated(&[(CONFIG_PATH_ENV, path.to_str().unwrap())]);
            let loaded = load(Flags::default()).map(|s| s.delivery);
            std::fs::remove_dir_all(&dir).ok();
            loaded
        };
        assert_eq!(mode("").unwrap(), DeliveryMode::Herdr);
        assert_eq!(
            mode("[watcher]\ninterval_secs = 3\n").unwrap(),
            DeliveryMode::Herdr
        );
        assert_eq!(
            mode("[watcher]\ndelivery = \"herdr\"\n").unwrap(),
            DeliveryMode::Herdr
        );
        assert_eq!(
            mode("[watcher]\ndelivery = \"socket\"\n").unwrap(),
            DeliveryMode::Socket
        );
        assert!(
            mode("[watcher]\ndelivery = \"uds\"\n").is_err(),
            "an unknown mode is refused"
        );
        assert!(mode("[watcher]\ndelivery = true\n").is_err());
    }

    /// Tracing: TC-044-01
    /// ACs: FR-044-AC-1
    ///
    /// `delivery` follows the rule of the metrics keys: the shared file loads
    /// whatever it holds, so a typo never stops `agent-msg` or a hook; only the watcher
    /// (and doctor's reading of it) refuses it, at its real line.
    #[test]
    fn tc_044_01_a_bad_delivery_value_loads_for_every_binary_and_only_the_watcher_refuses_it() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = scratch_dir("delivery-tolerated");
        for text in [
            "[watcher]\ndelivery = \"uds\"\n",
            "[watcher]\ndelivery = true\n",
            "[watcher]\ndelivery = 5\n",
        ] {
            let path = write_config(&dir, text);
            let _env = EnvGuard::isolated(&[(CONFIG_PATH_ENV, path.to_str().unwrap())]);
            assert!(load_file().is_ok(), "{text:?} must load for every binary");
            assert!(load_core(CoreFlags::default()).is_ok(), "{text:?}");
            let problem = watcher_delivery().unwrap_err();
            assert!(problem.contains("config.toml:2:"), "{text:?}: {problem}");
            let error = load(Flags::default()).unwrap_err();
            let ConfigError::Malformed { line, .. } = &error else {
                panic!("{text:?}: expected Malformed, got {error}");
            };
            assert_eq!(*line, 2, "{error}");
        }
        let path = write_config(&dir, "[watcher]\ndelivery = \"socket\"\n");
        let _env = EnvGuard::isolated(&[(CONFIG_PATH_ENV, path.to_str().unwrap())]);
        assert_eq!(watcher_delivery(), Ok(DeliveryMode::Socket));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unknown_watcher_key_is_refused() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = scratch_dir("unknown-watcher-key");
        let path = write_config(&dir, "[watcher]\ninterval_secs = 2\nbogus = 1\n");
        let _env = EnvGuard::isolated(&[(CONFIG_PATH_ENV, path.to_str().unwrap())]);

        let error = load_file().unwrap_err();
        assert!(matches!(error, ConfigError::Malformed { .. }), "{error}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_present_file_is_used_for_every_key() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = scratch_dir("present");
        let path = write_config(
            &dir,
            "url = \"redis://file:1\"\nmachine = \"File-Machine\"\nprefix = \"file:\"\nhuman = \"file-human\"\n\n[watcher]\ninterval_secs = 9\n",
        );
        let _env = EnvGuard::isolated(&[(CONFIG_PATH_ENV, path.to_str().unwrap())]);

        let settings = load(Flags::default()).unwrap();
        assert_eq!(settings.core.url.value, "redis://file:1");
        assert_eq!(settings.core.url.source, Source::File);
        assert_eq!(settings.core.machine.value.as_str(), "file-machine");
        assert_eq!(settings.core.machine.source, Source::File);
        assert_eq!(settings.core.prefix.value, "file:");
        assert_eq!(settings.core.human.value, "file-human");
        assert_eq!(settings.core.human.source, Source::File);
        assert_eq!(settings.watcher_interval_secs.value, 9);
        assert_eq!(settings.watcher_interval_secs.source, Source::File);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn env_beats_file_for_every_key() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = scratch_dir("env-beats-file");
        let path = write_config(
            &dir,
            "url = \"redis://file:1\"\nmachine = \"file-machine\"\nprefix = \"file:\"\nhuman = \"file-human\"\n\n[watcher]\ninterval_secs = 9\n",
        );
        let _env = EnvGuard::isolated(&[
            (CONFIG_PATH_ENV, path.to_str().unwrap()),
            (ENV_URL, "redis://env:2"),
            (ENV_MACHINE, "Env-Machine"),
            (ENV_PREFIX, "env:"),
            (ENV_HUMAN, "env-human"),
            (ENV_WATCHER_INTERVAL_SECS, "7"),
        ]);

        let settings = load(Flags::default()).unwrap();
        assert_eq!(settings.core.url.value, "redis://env:2");
        assert_eq!(settings.core.url.source, Source::Env);
        assert_eq!(settings.core.machine.value.as_str(), "env-machine");
        assert_eq!(settings.core.machine.source, Source::Env);
        assert_eq!(settings.core.prefix.value, "env:");
        assert_eq!(settings.core.human.value, "env-human");
        assert_eq!(settings.core.human.source, Source::Env);
        assert_eq!(settings.watcher_interval_secs.value, 7);
        assert_eq!(settings.watcher_interval_secs.source, Source::Env);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn flag_beats_env_and_file_for_every_key() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = scratch_dir("flag-beats-all");
        let path = write_config(
            &dir,
            "url = \"redis://file:1\"\nmachine = \"file-machine\"\nprefix = \"file:\"\nhuman = \"file-human\"\n\n[watcher]\ninterval_secs = 9\n",
        );
        let _env = EnvGuard::isolated(&[
            (CONFIG_PATH_ENV, path.to_str().unwrap()),
            (ENV_URL, "redis://env:2"),
            (ENV_MACHINE, "env-machine"),
            (ENV_PREFIX, "env:"),
            (ENV_HUMAN, "env-human"),
            (ENV_WATCHER_INTERVAL_SECS, "7"),
        ]);

        let flags = Flags {
            core: CoreFlags {
                url: Some("redis://flag:3".to_owned()),
                prefix: Some("flag:".to_owned()),
                machine: Some("Flag-Machine".to_owned()),
                human: Some("flag-human".to_owned()),
            },
            watcher_interval_secs: Some(5),
        };
        let settings = load(flags).unwrap();
        assert_eq!(settings.core.url.value, "redis://flag:3");
        assert_eq!(settings.core.url.source, Source::Flag);
        assert_eq!(settings.core.machine.value.as_str(), "flag-machine");
        assert_eq!(settings.core.machine.source, Source::Flag);
        assert_eq!(settings.core.prefix.value, "flag:");
        assert_eq!(settings.core.human.value, "flag-human");
        assert_eq!(settings.core.human.source, Source::Flag);
        assert_eq!(settings.watcher_interval_secs.value, 5);
        assert_eq!(settings.watcher_interval_secs.source, Source::Flag);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn bad_watcher_interval_env_value_is_refused_not_skipped() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = scratch_dir("bad-env-int");
        std::fs::remove_dir_all(&dir).ok();
        let _env = EnvGuard::isolated(&[
            (CONFIG_PATH_ENV, dir.join("config.toml").to_str().unwrap()),
            (ENV_WATCHER_INTERVAL_SECS, "not-a-number"),
        ]);

        let error = load(Flags::default()).unwrap_err();
        assert!(matches!(error, ConfigError::BadEnvValue { .. }), "{error}");
    }

    fn otlp_of(env: &[(&'static str, &str)], file: Option<&str>) -> otlp::OtlpSettings {
        let dir = scratch_dir("otlp-combo");
        std::fs::remove_dir_all(&dir).ok();
        let path = match file {
            Some(text) => write_config(&dir, text),
            None => dir.join("config.toml"),
        };
        let mut all = vec![(CONFIG_PATH_ENV, path.to_str().unwrap())];
        all.extend_from_slice(env);
        let _env = EnvGuard::isolated(&all);
        let settings = load(Flags::default()).unwrap().watcher_otlp;
        std::fs::remove_dir_all(&dir).ok();
        settings
    }

    fn target_of(s: &otlp::OtlpSettings) -> Option<String> {
        s.endpoint.as_ref().map(|e| e.value.target())
    }

    fn header_names(s: &otlp::OtlpSettings) -> Vec<String> {
        s.headers
            .value
            .pairs()
            .iter()
            .map(|(k, _)| k.clone())
            .collect()
    }

    /// Tracing: TC-045-02
    /// ACs: FR-045-AC-2
    #[test]
    fn tc_045_02_relay_settings_beat_the_generic_variables_and_the_default_is_no_endpoint() {
        let _guard = ENV_LOCK.lock().unwrap();
        let none = otlp_of(&[], None);
        assert!(none.endpoint.is_none() && none.ignored.is_empty());
        assert!(none.headers.value.is_empty());
        assert_eq!(
            (none.interval_secs.value, none.interval_secs.source),
            (otlp::DEFAULT_INTERVAL_SECS, Source::Default)
        );
        let file = "[watcher]\notlp_endpoint = \"http://file-host:4318\"\notlp_headers = \"a=file\"\notlp_interval_secs = 60\n";
        let s = otlp_of(&[], Some(file));
        let e = s.endpoint.as_ref().unwrap();
        assert_eq!(
            (e.value.target().as_str(), e.source),
            ("file-host:4318", Source::File)
        );
        assert_eq!(e.value.path(), "/v1/metrics");
        assert_eq!(
            (s.headers.source, s.interval_secs.value),
            (Source::File, 60)
        );
        // The relay's env endpoint beats the file; the generic variables beat neither.
        let s = otlp_of(
            &[
                (otlp::ENV_RELAY_ENDPOINT, "http://relay-env:4318"),
                (otlp::ENV_RELAY_HEADERS, "a=relay"),
                (otlp::ENV_ENDPOINT, "http://generic:4318"),
                (otlp::ENV_HEADERS, "a=generic"),
                (otlp::ENV_INTERVAL_SECS, "10"),
                (otlp::ENV_SERVICE_NAME, "svc"),
                (otlp::ENV_RESOURCE_ATTRIBUTES, "team=relay"),
            ],
            Some(file),
        );
        let e = s.endpoint.as_ref().unwrap();
        assert_eq!(
            (e.value.target().as_str(), e.source, e.origin),
            (
                "relay-env:4318",
                Source::Env,
                Some(otlp::ENV_RELAY_ENDPOINT)
            )
        );
        assert_eq!(s.headers.value.pairs()[0].1, "relay");
        assert_eq!(
            (s.interval_secs.value, s.interval_secs.source),
            (10, Source::Env)
        );
        assert_eq!(s.service_name.as_deref(), Some("svc"));
        assert_eq!(
            s.resource_attributes,
            [("team".to_owned(), "relay".to_owned())]
        );
        // A set-but-empty variable is unset.
        let s = otlp_of(
            &[
                (otlp::ENV_RELAY_ENDPOINT, ""),
                (otlp::ENV_RELAY_HEADERS, ""),
                (otlp::ENV_INTERVAL_SECS, ""),
            ],
            Some(file),
        );
        assert_eq!(s.endpoint.unwrap().source, Source::File);
    }

    /// The generic variables are honoured only when no relay endpoint is set, only
    /// for an http endpoint with no protocol but http/json, headers only from the
    /// same generic tier, and every unusable inherited setting is ignored with a
    /// note, never an error.
    ///
    /// Tracing: TC-045-16
    /// ACs: FR-045-AC-16, FR-045-AC-2
    #[test]
    fn tc_045_16_generic_otel_variables_are_a_fallback_never_fatal_and_headers_follow_their_endpoint()
     {
        let _guard = ENV_LOCK.lock().unwrap();
        // Usable: base and as-written, with and without an explicit http/json protocol.
        let s = otlp_of(
            &[
                (otlp::ENV_ENDPOINT, "http://g:4318"),
                (otlp::ENV_HEADERS, "x=1"),
            ],
            None,
        );
        assert_eq!(
            (target_of(&s).as_deref(), header_names(&s)),
            (Some("g:4318"), vec!["x".to_owned()])
        );
        assert_eq!(s.endpoint.as_ref().unwrap().value.path(), "/v1/metrics");
        let s = otlp_of(
            &[
                (otlp::ENV_METRICS_ENDPOINT, "http://g:9/m"),
                (otlp::ENV_ENDPOINT, "http://other:1"),
                (otlp::ENV_PROTOCOL, "http/json"),
            ],
            None,
        );
        assert_eq!(
            (target_of(&s).as_deref(), s.endpoint.unwrap().value.path()),
            (Some("g:9"), "/m")
        );
        assert!(
            otlp_of(
                &[
                    (otlp::ENV_ENDPOINT, "http://g:1"),
                    (otlp::ENV_METRICS_PROTOCOL, "HTTP/JSON")
                ],
                None
            )
            .ignored
            .is_empty()
        );
        // Unusable: ignored with one note naming the variable, never an error.
        for (env, var, reason) in [
            (
                vec![(otlp::ENV_ENDPOINT, "https://g:4318")],
                otlp::ENV_ENDPOINT,
                "https",
            ),
            (
                vec![(otlp::ENV_METRICS_ENDPOINT, "https://g")],
                otlp::ENV_METRICS_ENDPOINT,
                "https",
            ),
            (
                vec![
                    (otlp::ENV_ENDPOINT, "http://g:4317"),
                    (otlp::ENV_PROTOCOL, "grpc"),
                ],
                otlp::ENV_ENDPOINT,
                "protocol",
            ),
            (
                vec![
                    (otlp::ENV_ENDPOINT, "http://g:4318"),
                    (otlp::ENV_METRICS_PROTOCOL, "http/protobuf"),
                ],
                otlp::ENV_ENDPOINT,
                "protocol",
            ),
            (
                vec![(otlp::ENV_ENDPOINT, "g:4317")],
                otlp::ENV_ENDPOINT,
                "not a usable",
            ),
            (
                vec![(otlp::ENV_ENDPOINT, "http://u:p@g")],
                otlp::ENV_ENDPOINT,
                "not a usable",
            ),
            (
                vec![
                    (otlp::ENV_ENDPOINT, "http://g:4318"),
                    (otlp::ENV_HEADERS, "novalue"),
                ],
                otlp::ENV_ENDPOINT,
                "headers",
            ),
        ] {
            let s = otlp_of(&env, None);
            assert!(s.endpoint.is_none(), "{env:?}");
            assert_eq!(s.ignored.len(), 1, "{env:?}");
            assert_eq!(s.ignored[0].setting, var);
            assert!(s.ignored[0].reason.contains(reason), "{}", s.ignored[0]);
            let shown = s.ignored[0].to_string();
            assert!(
                !shown.contains("u:p") && !shown.contains("novalue"),
                "{shown}"
            );
        }
        // A relay endpoint (env or file) means the generic ones are not consulted at all.
        let file = "[watcher]\notlp_endpoint = \"http://file:1\"\n";
        let s = otlp_of(
            &[
                (otlp::ENV_ENDPOINT, "https://g"),
                (otlp::ENV_HEADERS, "tok=generic"),
            ],
            Some(file),
        );
        assert_eq!(
            (
                target_of(&s).as_deref(),
                s.headers.value.is_empty(),
                s.ignored.len()
            ),
            (Some("file:1"), true, 0)
        );
        let s = otlp_of(
            &[
                (otlp::ENV_RELAY_ENDPOINT, "http://r:1"),
                (otlp::ENV_HEADERS, "tok=generic"),
            ],
            None,
        );
        assert!(
            s.headers.value.is_empty(),
            "generic credentials never go to a relay endpoint"
        );
        // Headers of another tier than the endpoint's are dropped, with a note.
        let with_headers =
            "[watcher]\notlp_endpoint = \"http://file:1\"\notlp_headers = \"a=file\"\n";
        let s = otlp_of(
            &[(otlp::ENV_RELAY_ENDPOINT, "http://r:1")],
            Some(with_headers),
        );
        assert!(s.headers.value.is_empty());
        assert_eq!(
            s.ignored.iter().map(|i| i.setting).collect::<Vec<_>>(),
            ["otlp_headers"]
        );
        let s = otlp_of(&[(otlp::ENV_RELAY_HEADERS, "a=env")], Some(with_headers));
        assert_eq!(header_names(&s), ["a"]);
        assert_eq!(
            s.headers.source,
            Source::File,
            "the file's endpoint takes the file's headers"
        );
        assert_eq!(
            s.ignored.iter().map(|i| i.setting).collect::<Vec<_>>(),
            [otlp::ENV_RELAY_HEADERS]
        );
        // The file endpoint and the generic headers: never combined.
        let s = otlp_of(
            &[(otlp::ENV_HEADERS, "tok=generic")],
            Some("[watcher]\notlp_endpoint = \"http://file:1\"\n"),
        );
        assert!(s.headers.value.is_empty());
    }

    /// Tracing: TC-045-14
    /// ACs: FR-045-AC-14
    #[test]
    fn tc_045_14_an_unusable_relay_otlp_value_is_refused_naming_its_source() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = scratch_dir("otlp-refused");
        // From the file: the file's own error, at the real line and column.
        for text in [
            "[watcher]\notlp_endpoint = 5\n",
            "[watcher]\notlp_endpoint = \"https://collector:4318\"\n",
            "[watcher]\notlp_endpoint = \"collector\"\n",
            "[watcher]\notlp_headers = 5\n",
            "[watcher]\notlp_headers = \"novalue\"\n",
            "[watcher]\notlp_headers = \"a=b%+a\"\n",
            "[watcher]\notlp_interval_secs = 1\n",
            "[watcher]\notlp_interval_secs = 3601\n",
            "[watcher]\notlp_interval_secs = 9223372036854775807\n",
            "[watcher]\notlp_interval_secs = -1\n",
            "[watcher]\notlp_interval_secs = \"abc\"\n",
        ] {
            let path = write_config(&dir, text);
            let _env = EnvGuard::isolated(&[(CONFIG_PATH_ENV, path.to_str().unwrap())]);
            // The shared file type tolerates them: only the watcher refuses.
            assert!(load_file().is_ok(), "{text:?} must load for every binary");
            assert!(load_core(CoreFlags::default()).is_ok(), "{text:?}");
            assert!(
                otlp::file_problem().is_some_and(|p| p.contains("config.toml:2:")),
                "{text:?}: doctor sees it with its position"
            );
            let error = load(Flags::default()).unwrap_err();
            let ConfigError::Malformed { path: at, line, .. } = &error else {
                panic!("{text:?}: expected Malformed, got {error}");
            };
            assert!(at.ends_with("config.toml"), "{error}");
            assert_eq!(*line, 2, "the real line of the key, not 1:1: {error}");
        }
        // The bounds themselves are accepted.
        for ok in ["5", "3600"] {
            let path = write_config(&dir, &format!("[watcher]\notlp_interval_secs = {ok}\n"));
            let _env = EnvGuard::isolated(&[(CONFIG_PATH_ENV, path.to_str().unwrap())]);
            assert!(otlp::file_problem().is_none(), "{ok}");
            assert_eq!(
                load(Flags::default())
                    .unwrap()
                    .watcher_otlp
                    .interval_secs
                    .value,
                ok.parse::<u64>().unwrap()
            );
        }
        // The https refusal names the key and says why.
        let path = write_config(&dir, "[watcher]\notlp_endpoint = \"https://c:1\"\n");
        let _env = EnvGuard::isolated(&[(CONFIG_PATH_ENV, path.to_str().unwrap())]);
        let error = load(Flags::default()).unwrap_err().to_string();
        assert!(
            error.contains("otlp_endpoint") && error.contains("no TLS"),
            "{error}"
        );
        // From the environment: the variable, never a file that does not exist.
        let missing = dir.join("absent.toml");
        for (name, value) in [
            (otlp::ENV_RELAY_ENDPOINT, "https://collector"),
            (otlp::ENV_RELAY_ENDPOINT, "http://u:pw@collector"),
            (otlp::ENV_RELAY_HEADERS, "novalue"),
            (otlp::ENV_INTERVAL_SECS, "4"),
            (otlp::ENV_INTERVAL_SECS, "3601"),
            (otlp::ENV_INTERVAL_SECS, "18446744073709551615"),
            (otlp::ENV_INTERVAL_SECS, "9223372036854775807"),
            (otlp::ENV_INTERVAL_SECS, "abc"),
        ] {
            let _env =
                EnvGuard::isolated(&[(CONFIG_PATH_ENV, missing.to_str().unwrap()), (name, value)]);
            let error = load(Flags::default()).unwrap_err();
            assert!(
                matches!(&error, ConfigError::BadEnvValue { name: n, .. } if *n == name),
                "{name}={value}: {error}"
            );
            let shown = error.to_string();
            assert!(
                !shown.contains("absent.toml") && !shown.contains("pw@"),
                "{shown}"
            );
            // agent-msg resolves none of these keys.
            assert!(load_core(CoreFlags::default()).is_ok());
        }
        // A header value never reaches an error.
        let _env = EnvGuard::isolated(&[
            (CONFIG_PATH_ENV, missing.to_str().unwrap()),
            (otlp::ENV_RELAY_HEADERS, "Authorization=s3cret-token,bad"),
        ]);
        let shown = load(Flags::default()).unwrap_err().to_string();
        assert!(!shown.contains("s3cret-token"), "{shown}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_core_ignores_a_bad_watcher_interval_env_value() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = scratch_dir("core-ignores-bad-watcher-env");
        std::fs::remove_dir_all(&dir).ok();
        let _env = EnvGuard::isolated(&[
            (CONFIG_PATH_ENV, dir.join("config.toml").to_str().unwrap()),
            (ENV_WATCHER_INTERVAL_SECS, "not-a-number"),
        ]);

        // agent-msg calls load_core, never load: a bad
        // IX_RELAY_WATCHER_INTERVAL_SECS must not be able to fail it, since
        // load_core never reads that key at all.
        let settings = load_core(CoreFlags::default()).unwrap();
        assert_eq!(settings.url.value, DEFAULT_URL);
    }

    #[test]
    fn empty_string_values_in_the_file_are_refused() {
        let _guard = ENV_LOCK.lock().unwrap();
        for key in ["url", "machine", "prefix", "human"] {
            let dir = scratch_dir(&format!("empty-{key}"));
            let path = write_config(&dir, &format!("{key} = \"\"\n"));
            let _env = EnvGuard::isolated(&[(CONFIG_PATH_ENV, path.to_str().unwrap())]);

            let error = load_file().unwrap_err();
            let ConfigError::Malformed { message, .. } = &error else {
                panic!("{key}: expected Malformed, got {error}");
            };
            assert!(
                message.contains(key) && message.contains("empty"),
                "{key}: {error}"
            );
            std::fs::remove_dir_all(&dir).ok();
        }
    }

    #[test]
    fn empty_env_values_fall_through_to_the_file() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = scratch_dir("empty-env");
        let path = write_config(
            &dir,
            "url = \"redis://file:1\"\nmachine = \"file-machine\"\nprefix = \"file:\"\n",
        );
        let _env = EnvGuard::isolated(&[
            (CONFIG_PATH_ENV, path.to_str().unwrap()),
            (ENV_URL, ""),
            (ENV_MACHINE, ""),
            (ENV_PREFIX, ""),
        ]);

        let settings = load_core(CoreFlags::default()).unwrap();
        assert_eq!(settings.url.value, "redis://file:1");
        assert_eq!(settings.url.source, Source::File);
        assert_eq!(settings.machine.value.as_str(), "file-machine");
        assert_eq!(settings.machine.source, Source::File);
        assert_eq!(settings.prefix.value, "file:");
        assert_eq!(settings.prefix.source, Source::File);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_machine_that_normalizes_to_empty_is_refused() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = scratch_dir("empty-normalized-machine");
        std::fs::remove_dir_all(&dir).ok();
        let _env =
            EnvGuard::isolated(&[(CONFIG_PATH_ENV, dir.join("config.toml").to_str().unwrap())]);

        // "." normalizes to "" (split on the first '.'): a raw value that is
        // itself non-empty but resolves to an empty machine name must still
        // be refused, regardless of which tier supplied it — here, the flag.
        let flags = CoreFlags {
            machine: Some(".".to_owned()),
            ..CoreFlags::default()
        };
        let error = load_core(flags).unwrap_err();
        let ConfigError::Malformed { message, .. } = &error else {
            panic!("expected Malformed, got {error}");
        };
        assert!(
            message.contains("machine") && message.contains("empty"),
            "{error}"
        );
    }

    #[test]
    fn config_dir_is_the_config_files_directory() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _env = EnvGuard::isolated(&[("IX_RELAY_CONFIG", "/somewhere/else/config.toml")]);
        assert_eq!(config_dir(), Some(PathBuf::from("/somewhere/else")));
    }

    #[test]
    fn config_path_prefers_the_override_env_var() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _env = EnvGuard::isolated(&[(CONFIG_PATH_ENV, "/tmp/somewhere/config.toml")]);
        assert_eq!(
            config_path(),
            Some(PathBuf::from("/tmp/somewhere/config.toml"))
        );
    }

    /// Tracing: FR-026 (Behavior: `IX_RELAY_CONFIG` selects where the machine's files live). No acceptance criterion row covers it; the state file itself has no owning requirement.
    #[test]
    fn state_dir_sits_beside_the_config_file() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _env = EnvGuard::isolated(&[(CONFIG_PATH_ENV, "/tmp/somewhere/config.toml")]);
        assert_eq!(state_dir(), Some(PathBuf::from("/tmp/somewhere/state")));
    }

    #[test]
    fn line_col_counts_characters_not_bytes_and_handles_crlf() {
        // "café\r\nbar": 'é' is 2 UTF-8 bytes, and the line break is CRLF.
        // The offset of 'b' (start of line 2) must land at column 1 on line
        // 2 — a byte-counting implementation would report a different
        // column because of 'é''s extra byte.
        let text = "café\r\nbar";
        let b_offset = text.find('b').unwrap();
        assert_eq!(line_col(text, b_offset), (2, 1));

        // A byte offset that lands inside 'é' (not a char boundary) must not
        // panic — see this function's doc comment.
        let mid_e_acute = text.find('é').unwrap() + 1;
        assert_eq!(line_col(text, mid_e_acute), (1, 5));
    }

    #[test]
    fn redact_url_hides_userinfo_and_leaves_the_rest() {
        for (raw, shown) in [
            (
                "redis://user:hunter2@host:6379/0",
                "redis://***@host:6379/0",
            ),
            ("redis://:hunter2@host", "redis://***@host"),
            ("rediss://a:b@h/?x=y@z", "rediss://***@z"),
            ("redis://u:p/a?s#s@host:6379/0", "redis://***@host:6379/0"),
            ("redis://host:6379", "redis://host:6379"),
            ("redis://127.0.0.1:6379/1", "redis://127.0.0.1:6379/1"),
            ("not a url", "not a url"),
        ] {
            assert_eq!(redact_url(raw), shown, "{raw}");
        }
    }

    proptest::proptest! {
        /// Whatever the password holds, it never survives redaction.
        #[test]
        fn redact_url_never_leaks_the_password(
            user in "[a-z]{0,6}", pass in "[A-Za-z][A-Za-z0-9._~!$&*+/?#@-]{7,15}", host in "[a-z]{1,4}",
        ) {
            let redacted = redact_url(&format!("redis://{user}:{pass}@{host}:6379/0"));
            let tail = format!("@{host}:6379/0");
            proptest::prop_assert!(!redacted.contains(&pass));
            proptest::prop_assert!(redacted.ends_with(&tail));
        }
    }
}
