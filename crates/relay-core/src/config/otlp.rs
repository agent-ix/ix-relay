//! The watcher's OTLP settings (FR-045): `[watcher] otlp_endpoint`, `otlp_headers`
//! and `otlp_interval_secs`, and the standard OpenTelemetry environment variables.
//!
//! These are watcher-only keys, so the shared [`super::WatcherFile`] holds them as
//! raw TOML values and never refuses one: a typo must not stop `agent-msg` or a
//! hook. [`resolve`] validates them, and only `relay-watcher` calls it at start-up;
//! [`file_problem`] is the same validation over the file alone, for `agent-msg
//! doctor`. Both refuse the same values, so the doctor never warns about something
//! the watcher would accept, or the reverse.
//!
//! Nothing here opens a socket. An endpoint is parsed into an [`OtlpEndpoint`]
//! (`http://` only: no TLS library is in the tree, so `https://` is refused with a
//! message naming the key or variable that supplied it).
//!
//! Header values are credentials. [`OtlpHeaders`] never prints them, and an error
//! about a header never repeats one.

use std::fmt;

use ix_cli_kit::config::{self, ConfigError, ParseLocation, Resolved, Source};
use serde::Deserialize;
use toml::Spanned;

use super::{FileConfig, config_path, line_col, non_empty_env, redact_url, toml_error_parts};

/// `IX_RELAY_WATCHER_OTLP_ENDPOINT`: the relay's own endpoint variable, a base URL
/// like the file key. It and the file key are the primary settings.
pub const ENV_RELAY_ENDPOINT: &str = "IX_RELAY_WATCHER_OTLP_ENDPOINT";
/// `IX_RELAY_WATCHER_OTLP_HEADERS`: request headers for [`ENV_RELAY_ENDPOINT`].
pub const ENV_RELAY_HEADERS: &str = "IX_RELAY_WATCHER_OTLP_HEADERS";
/// The generic `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT` (other emitters on the host,
/// Claude Code among them, are configured through it): the full URL metrics are
/// posted to. Honoured only when no relay-specific endpoint is configured.
pub const ENV_METRICS_ENDPOINT: &str = "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT";
/// The generic `OTEL_EXPORTER_OTLP_ENDPOINT`: the base URL; `/v1/metrics` is
/// appended. Honoured only when no relay-specific endpoint is configured.
pub const ENV_ENDPOINT: &str = "OTEL_EXPORTER_OTLP_ENDPOINT";
/// The generic `OTEL_EXPORTER_OTLP_HEADERS`: `key=value,key=value` request headers,
/// sent only to an endpoint that came from the same generic variables.
pub const ENV_HEADERS: &str = "OTEL_EXPORTER_OTLP_HEADERS";
/// `OTEL_EXPORTER_OTLP_PROTOCOL`: `grpc`, `http/protobuf` or `http/json`.
pub const ENV_PROTOCOL: &str = "OTEL_EXPORTER_OTLP_PROTOCOL";
/// `OTEL_EXPORTER_OTLP_METRICS_PROTOCOL`: the metrics-specific protocol.
pub const ENV_METRICS_PROTOCOL: &str = "OTEL_EXPORTER_OTLP_METRICS_PROTOCOL";
/// `OTEL_RESOURCE_ATTRIBUTES`: `key=value,key=value` resource attributes.
pub const ENV_RESOURCE_ATTRIBUTES: &str = "OTEL_RESOURCE_ATTRIBUTES";
/// `OTEL_SERVICE_NAME`: the `service.name` resource attribute.
pub const ENV_SERVICE_NAME: &str = "OTEL_SERVICE_NAME";
/// `IX_RELAY_WATCHER_OTLP_INTERVAL_SECS`: seconds between pushes.
pub const ENV_INTERVAL_SECS: &str = "IX_RELAY_WATCHER_OTLP_INTERVAL_SECS";

/// The path OTLP/HTTP appends to a base endpoint for metrics.
pub const METRICS_PATH: &str = "/v1/metrics";

/// Seconds between pushes when nothing sets it.
pub const DEFAULT_INTERVAL_SECS: u64 = 30;
/// The shortest push interval accepted.
pub const MIN_INTERVAL_SECS: u64 = 5;
/// The longest push interval accepted: an hour. A larger value is refused, so
/// time arithmetic on the interval cannot overflow.
pub const MAX_INTERVAL_SECS: u64 = 3600;

/// The most request headers accepted.
pub const MAX_HEADERS: usize = 16;
/// Header names the exporter sets itself, which a configured header may not
/// override.
pub const RESERVED_HEADERS: [&str; 5] = [
    "host",
    "content-length",
    "content-type",
    "connection",
    "transfer-encoding",
];

/// The most bytes of one header name or value accepted.
pub const MAX_HEADER_BYTES: usize = 1024;
/// The most resource attributes taken from `OTEL_RESOURCE_ATTRIBUTES`.
pub const MAX_RESOURCE_ATTRIBUTES: usize = 32;
/// The longest endpoint text accepted, in bytes.
pub const MAX_ENDPOINT_BYTES: usize = 2048;

/// Why an endpoint is unusable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EndpointError {
    /// Nothing, or only whitespace.
    #[error("the endpoint is empty")]
    Empty,
    /// The text is longer than [`MAX_ENDPOINT_BYTES`].
    #[error("the endpoint is longer than {MAX_ENDPOINT_BYTES} bytes")]
    TooLong,
    /// `https://`: this build has no TLS library, so it cannot speak it.
    #[error(
        "https endpoints are not supported: this build sends OTLP over plain http only \
         (no TLS library); point it at an http:// collector"
    )]
    Https,
    /// Not an `http://` URL.
    #[error("the endpoint must be an http:// URL")]
    NotHttp,
    /// `user:password@` in the URL.
    #[error("credentials in the endpoint are not supported; put them in the headers setting")]
    Credentials,
    /// A query string or fragment.
    #[error("the endpoint must not have a query string or fragment")]
    QueryOrFragment,
    /// A control character, space or other character a URL cannot hold.
    #[error("the endpoint holds a character a URL cannot")]
    BadCharacter,
    /// No host, or a bracketed address that does not close.
    #[error("the endpoint has no usable host")]
    BadHost,
    /// A port that is not 1 to 65535.
    #[error("the endpoint's port must be 1 to 65535")]
    BadPort,
}

/// Where the watcher posts metrics: an `http://` collector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtlpEndpoint {
    host: String,
    port: u16,
    path: String,
}

impl OtlpEndpoint {
    /// Parses `text`. `base` is true for a base URL (the file key and
    /// `OTEL_EXPORTER_OTLP_ENDPOINT`), which gets [`METRICS_PATH`] appended;
    /// false for `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT`, used as written.
    ///
    /// # Errors
    ///
    /// An [`EndpointError`] naming what is wrong.
    pub fn parse(text: &str, base: bool) -> Result<Self, EndpointError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(EndpointError::Empty);
        }
        if text.len() > MAX_ENDPOINT_BYTES {
            return Err(EndpointError::TooLong);
        }
        let (scheme, rest) = text.split_once("://").ok_or(EndpointError::NotHttp)?;
        if scheme.eq_ignore_ascii_case("https") {
            return Err(EndpointError::Https);
        }
        if !scheme.eq_ignore_ascii_case("http") {
            return Err(EndpointError::NotHttp);
        }
        if rest
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || !c.is_ascii())
        {
            return Err(EndpointError::BadCharacter);
        }
        let (authority, path) = rest.find('/').map_or((rest, ""), |i| rest.split_at(i));
        if authority.contains('@') {
            return Err(EndpointError::Credentials);
        }
        if path.contains(['?', '#']) || authority.contains(['?', '#']) {
            return Err(EndpointError::QueryOrFragment);
        }
        let (host, port) = split_host_port(authority)?;
        let path = if base {
            format!("{}{METRICS_PATH}", path.trim_end_matches('/'))
        } else if path.is_empty() {
            "/".to_owned()
        } else {
            path.to_owned()
        };
        Ok(Self { host, port, path })
    }

    /// The host to connect to: a name or an address, without brackets.
    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The TCP port.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The request target, always starting with `/`.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// `host:port`, as shown in logs, the heartbeat and `doctor`. Holds no
    /// credential, query or path.
    #[must_use]
    pub fn target(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

fn split_host_port(authority: &str) -> Result<(String, u16), EndpointError> {
    let (host, port) = if let Some(inner) = authority.strip_prefix('[') {
        let (host, after) = inner.split_once(']').ok_or(EndpointError::BadHost)?;
        (host, after.strip_prefix(':'))
    } else if let Some((host, port)) = authority.rsplit_once(':') {
        (host, Some(port))
    } else {
        (authority, None)
    };
    if host.is_empty()
        || host.contains(['[', ']'])
        || (!authority.starts_with('[') && host.contains(':'))
    {
        return Err(EndpointError::BadHost);
    }
    let port = match port {
        None => 80,
        Some(text) => text
            .parse::<u16>()
            .ok()
            .filter(|port| *port != 0)
            .ok_or(EndpointError::BadPort)?,
    };
    Ok((host.to_owned(), port))
}

/// Request headers for the collector. Values are credentials: `Debug` and
/// `Display` show names only.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct OtlpHeaders(Vec<(String, String)>);

impl OtlpHeaders {
    /// Parses `key=value,key=value` (the `OTEL_EXPORTER_OTLP_HEADERS` format;
    /// values may be percent-encoded).
    ///
    /// # Errors
    ///
    /// A message that names the position of the bad pair and never its value.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut out = Vec::new();
        for (index, pair) in text.split(',').enumerate() {
            let pair = pair.trim();
            if pair.is_empty() {
                continue;
            }
            let position = index + 1;
            let (name, value) = pair
                .split_once('=')
                .ok_or_else(|| format!("header {position} is not name=value"))?;
            let name = name.trim();
            let value = percent_decode(value.trim())
                .ok_or_else(|| format!("header {position} has a bad percent escape"))?;
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            {
                return Err(format!("header {position} has an invalid name"));
            }
            if RESERVED_HEADERS
                .iter()
                .any(|r| name.eq_ignore_ascii_case(r))
            {
                return Err(format!("header {position} is set by the exporter itself"));
            }
            if value.chars().any(char::is_control) {
                return Err(format!(
                    "header {position} has a control character in its value"
                ));
            }
            if name.len() > MAX_HEADER_BYTES || value.len() > MAX_HEADER_BYTES {
                return Err(format!(
                    "header {position} is longer than {MAX_HEADER_BYTES} bytes"
                ));
            }
            out.push((name.to_owned(), value));
        }
        if out.len() > MAX_HEADERS {
            return Err(format!("more than {MAX_HEADERS} headers"));
        }
        Ok(Self(out))
    }

    /// The `(name, value)` pairs, for the request. The values are secret.
    #[must_use]
    pub fn pairs(&self) -> &[(String, String)] {
        &self.0
    }

    /// Whether there are none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for OtlpHeaders {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.0.iter().map(|(name, _)| format!("{name}=<redacted>")))
            .finish()
    }
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if b == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            // Two hex digits and nothing else: `from_str_radix` alone would read
            // `+a` as 10.
            if !hex.bytes().all(|d| d.is_ascii_hexdigit()) {
                return None;
            }
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// `key=value,key=value` resource attributes. A pair without `=` or with an empty
/// key is skipped, as the OpenTelemetry specification says; the count of skipped
/// pairs is returned so the watcher can say so. At most
/// [`MAX_RESOURCE_ATTRIBUTES`] are kept.
#[must_use]
pub fn parse_resource_attributes(text: &str) -> (Vec<(String, String)>, usize) {
    let mut kept = Vec::new();
    let mut skipped = 0;
    for pair in text.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        match pair.split_once('=') {
            Some((key, value))
                if !key.trim().is_empty() && kept.len() < MAX_RESOURCE_ATTRIBUTES =>
            {
                kept.push((
                    key.trim().to_owned(),
                    percent_decode(value.trim()).unwrap_or_else(|| value.trim().to_owned()),
                ));
            }
            _ => skipped += 1,
        }
    }
    (kept, skipped)
}

/// The watcher's resolved OTLP settings.
#[derive(Debug, Clone)]
pub struct OtlpSettings {
    /// Where to post; `None` means nothing is ever sent.
    pub endpoint: Option<Resolved<OtlpEndpoint>>,
    /// Request headers (secret values).
    pub headers: Resolved<OtlpHeaders>,
    /// Seconds between pushes, from [`MIN_INTERVAL_SECS`] to [`MAX_INTERVAL_SECS`].
    pub interval_secs: Resolved<u64>,
    /// `OTEL_RESOURCE_ATTRIBUTES`, in order.
    pub resource_attributes: Vec<(String, String)>,
    /// How many of its pairs were skipped as malformed.
    pub resource_attributes_skipped: usize,
    /// `OTEL_SERVICE_NAME`, when set.
    pub service_name: Option<String>,
    /// Settings that were present but not used: inherited generic OpenTelemetry
    /// variables this exporter cannot honour, and headers of another tier than the
    /// endpoint. Never fatal; the watcher logs each once and the heartbeat carries
    /// the first.
    pub ignored: Vec<Ignored>,
}

/// A setting that was present and not used, and why. Names a variable or key and a
/// fixed reason, never a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ignored {
    /// The variable or file key.
    pub setting: &'static str,
    /// Why, as a fixed phrase.
    pub reason: &'static str,
}

impl std::fmt::Display for Ignored {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ignored: {}", self.setting, self.reason)
    }
}

/// The file's watcher-only OTLP keys as their types read them, each with its span.
#[derive(Deserialize, Default)]
struct StrictWatcher {
    otlp_endpoint: Option<Spanned<String>>,
    otlp_headers: Option<Spanned<String>>,
    otlp_interval_secs: Option<Spanned<u64>>,
}

#[derive(Deserialize)]
struct StrictFile {
    #[serde(default)]
    watcher: StrictWatcher,
}

/// The file's values, validated: endpoint (base URL), headers, interval.
type FileValues = (Option<OtlpEndpoint>, Option<OtlpHeaders>, Option<u64>);

fn malformed(path: &str, text: &str, offset: usize, key: &str, message: &str) -> ConfigError {
    let (line, column) = line_col(text, offset);
    ConfigError::Malformed {
        path: path.to_owned(),
        line,
        column,
        message: format!("{key}: {message}"),
    }
}

/// The file's OTLP keys, validated. Only reads the file again when it holds one,
/// and then reports the problem at its real line and column.
fn from_file(file: &FileConfig) -> Result<FileValues, ConfigError> {
    let w = &file.watcher;
    if w.otlp_endpoint.is_none() && w.otlp_headers.is_none() && w.otlp_interval_secs.is_none() {
        return Ok((None, None, None));
    }
    let Some(path) = config_path() else {
        return Ok((None, None, None));
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        // Unreadable now though it loaded a moment ago: nothing to validate.
        return Ok((None, None, None));
    };
    let shown = path.display().to_string();
    let strict = match toml::from_str::<StrictFile>(&text) {
        Ok(strict) => strict.watcher,
        Err(error) => {
            let (ParseLocation { line, column }, message) = toml_error_parts(&text, &error);
            let key = text
                .lines()
                .nth(line.saturating_sub(1))
                .and_then(|l| l.split('=').next())
                .map_or("", str::trim);
            return Err(ConfigError::Malformed {
                path: shown,
                line,
                column,
                message: if key.is_empty() {
                    message
                } else {
                    format!("{key}: {message}")
                },
            });
        }
    };
    let endpoint = strict
        .otlp_endpoint
        .map(|v| {
            OtlpEndpoint::parse(v.get_ref(), true).map_err(|e| {
                malformed(
                    &shown,
                    &text,
                    v.span().start,
                    "otlp_endpoint",
                    &e.to_string(),
                )
            })
        })
        .transpose()?;
    let headers = strict
        .otlp_headers
        .map(|v| {
            OtlpHeaders::parse(v.get_ref())
                .map_err(|e| malformed(&shown, &text, v.span().start, "otlp_headers", &e))
        })
        .transpose()?;
    let interval = strict
        .otlp_interval_secs
        .map(|v| {
            let secs = *v.get_ref();
            if !(MIN_INTERVAL_SECS..=MAX_INTERVAL_SECS).contains(&secs) {
                Err(malformed(
                    &shown,
                    &text,
                    v.span().start,
                    "otlp_interval_secs",
                    &format!("must be from {MIN_INTERVAL_SECS} to {MAX_INTERVAL_SECS} seconds"),
                ))
            } else {
                Ok(secs)
            }
        })
        .transpose()?;
    Ok((endpoint, headers, interval))
}

/// Why the watcher would refuse this machine's file for its OTLP keys, if it
/// would: what `agent-msg doctor` reports as `watcher_otlp_config_invalid`. The
/// file only: the watcher's own environment is not the doctor's. `None` when the
/// keys are usable or the file cannot be loaded at all (`config_invalid` says
/// that).
#[must_use]
pub fn file_problem() -> Option<String> {
    let file = super::load_file().ok()?.unwrap_or(FileConfig::default());
    from_file(&file).err().map(|e| e.to_string())
}

fn bad_env(name: &'static str, value: &str, message: impl Into<String>) -> ConfigError {
    ConfigError::BadEnvValue {
        name,
        // A header value is a credential and an endpoint may carry one by mistake.
        value: if name == ENV_HEADERS || name == ENV_RELAY_HEADERS {
            "<redacted>".to_owned()
        } else {
            redact_url(value)
        },
        message: message.into(),
    }
}

/// The generic OpenTelemetry endpoint and headers, when they are usable: `Ok(None)`
/// when none is set, `Err` with the reason it is unusable. Never fatal: these
/// variables belong to other emitters.
fn generic() -> Result<Option<(&'static str, OtlpEndpoint, OtlpHeaders)>, Ignored> {
    let (name, text, base) = if let Some(text) = non_empty_env(ENV_METRICS_ENDPOINT) {
        (ENV_METRICS_ENDPOINT, text, false)
    } else if let Some(text) = non_empty_env(ENV_ENDPOINT) {
        (ENV_ENDPOINT, text, true)
    } else {
        return Ok(None);
    };
    let ignore = |reason| Ignored {
        setting: name,
        reason,
    };
    // Only OTLP/HTTP JSON is spoken here: a gRPC or protobuf-only setting is a
    // setting for another emitter. Unset means the endpoint alone was given.
    let protocol = non_empty_env(ENV_METRICS_PROTOCOL).or_else(|| non_empty_env(ENV_PROTOCOL));
    if protocol.is_some_and(|p| !p.trim().eq_ignore_ascii_case("http/json")) {
        return Err(ignore("its protocol is not http/json"));
    }
    let endpoint = OtlpEndpoint::parse(&text, base).map_err(|e| {
        ignore(match e {
            EndpointError::Https => "https is not supported (plain http only)",
            _ => "it is not a usable http:// URL",
        })
    })?;
    let headers = match non_empty_env(ENV_HEADERS) {
        Some(text) => OtlpHeaders::parse(&text).map_err(|_| ignore("its headers are malformed"))?,
        None => OtlpHeaders::default(),
    };
    Ok(Some((name, endpoint, headers)))
}

/// Parses the interval from its variable, from [`MIN_INTERVAL_SECS`] to
/// [`MAX_INTERVAL_SECS`].
fn env_interval() -> Result<Option<(&'static str, u64)>, ConfigError> {
    non_empty_env(ENV_INTERVAL_SECS)
        .map(|text| {
            let secs = text
                .parse::<u64>()
                .map_err(|e| bad_env(ENV_INTERVAL_SECS, &text, e.to_string()))?;
            if (MIN_INTERVAL_SECS..=MAX_INTERVAL_SECS).contains(&secs) {
                Ok((ENV_INTERVAL_SECS, secs))
            } else {
                Err(bad_env(
                    ENV_INTERVAL_SECS,
                    &text,
                    format!("must be from {MIN_INTERVAL_SECS} to {MAX_INTERVAL_SECS} seconds"),
                ))
            }
        })
        .transpose()
}

/// Resolves the OTLP settings. The relay's own settings come first: the endpoint
/// from `IX_RELAY_WATCHER_OTLP_ENDPOINT`, else the `[watcher]` file key, and each
/// is validated whether or not it is used. Only when neither is set are the generic
/// `OTEL_EXPORTER_OTLP_*` variables consulted, and only if they name an `http://`
/// endpoint and no other protocol than http/json; otherwise they are ignored (see
/// [`OtlpSettings::ignored`]), never fatal. Headers come from the same tier as the
/// endpoint and no other, so credentials meant for one collector are never sent to
/// another. The interval is the variable, else the file, else 30 s. Only the watcher
/// calls this, so a bad relay value stops the watcher and nothing else, naming the
/// variable, or the file's own line and column.
///
/// # Errors
///
/// [`ConfigError::BadEnvValue`] for a relay variable, [`ConfigError::Malformed`] for
/// the file.
pub fn resolve(file: &FileConfig) -> Result<OtlpSettings, ConfigError> {
    let (file_endpoint, file_headers, file_interval) = from_file(file)?;
    let relay_endpoint = non_empty_env(ENV_RELAY_ENDPOINT)
        .map(|text| {
            OtlpEndpoint::parse(&text, true)
                .map_err(|e| bad_env(ENV_RELAY_ENDPOINT, &text, e.to_string()))
        })
        .transpose()?;
    let relay_headers = non_empty_env(ENV_RELAY_HEADERS)
        .map(|text| OtlpHeaders::parse(&text).map_err(|e| bad_env(ENV_RELAY_HEADERS, &text, e)))
        .transpose()?;
    let interval_secs =
        config::resolve(None, env_interval()?, file_interval, DEFAULT_INTERVAL_SECS);

    let mut ignored = Vec::new();
    let mut leftover_relay_headers = relay_headers.is_some();
    let mut leftover_file_headers = file_headers.is_some();
    let (endpoint, headers) = if let Some(value) = relay_endpoint {
        leftover_relay_headers = false;
        (
            Some(Resolved {
                value,
                source: Source::Env,
                origin: Some(ENV_RELAY_ENDPOINT),
            }),
            Resolved {
                value: relay_headers.clone().unwrap_or_default(),
                source: Source::Env,
                origin: Some(ENV_RELAY_HEADERS),
            },
        )
    } else if let Some(value) = file_endpoint {
        leftover_file_headers = false;
        (
            Some(Resolved {
                value,
                source: Source::File,
                origin: None,
            }),
            Resolved {
                value: file_headers.clone().unwrap_or_default(),
                source: Source::File,
                origin: None,
            },
        )
    } else {
        match generic() {
            Ok(Some((name, value, headers))) => (
                Some(Resolved {
                    value,
                    source: Source::Env,
                    origin: Some(name),
                }),
                Resolved {
                    value: headers,
                    source: Source::Env,
                    origin: Some(ENV_HEADERS),
                },
            ),
            Ok(None) => (
                None,
                config::resolve(None, None, None, OtlpHeaders::default()),
            ),
            Err(note) => {
                ignored.push(note);
                (
                    None,
                    config::resolve(None, None, None, OtlpHeaders::default()),
                )
            }
        }
    };
    // Relay headers of a tier other than the endpoint's are never sent to it.
    if leftover_relay_headers {
        ignored.push(Ignored {
            setting: ENV_RELAY_HEADERS,
            reason: "there is no relay endpoint in the environment to send them to",
        });
    }
    if leftover_file_headers {
        ignored.push(Ignored {
            setting: "otlp_headers",
            reason: "the endpoint comes from another tier than the file",
        });
    }

    let (resource_attributes, resource_attributes_skipped) = non_empty_env(ENV_RESOURCE_ATTRIBUTES)
        .map_or((Vec::new(), 0), |text| parse_resource_attributes(&text));
    Ok(OtlpSettings {
        endpoint,
        headers,
        interval_secs,
        resource_attributes,
        resource_attributes_skipped,
        service_name: non_empty_env(ENV_SERVICE_NAME),
        ignored,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tracing: TC-045-02
    /// ACs: FR-045-AC-2
    #[test]
    fn tc_045_02_a_base_endpoint_gets_the_metrics_path_and_a_signal_endpoint_is_used_as_is() {
        let base = OtlpEndpoint::parse("http://collector:4318", true).unwrap();
        assert_eq!(
            (base.host(), base.port(), base.path()),
            ("collector", 4318, "/v1/metrics")
        );
        let nested = OtlpEndpoint::parse("http://collector:4318/otel/", true).unwrap();
        assert_eq!(nested.path(), "/otel/v1/metrics");
        let signal = OtlpEndpoint::parse("http://[::1]:9/custom/path", false).unwrap();
        assert_eq!(
            (signal.host(), signal.port(), signal.path()),
            ("::1", 9, "/custom/path")
        );
        assert_eq!(signal.target(), "[::1]:9");
        assert_eq!(OtlpEndpoint::parse("http://c", false).unwrap().path(), "/");
        assert_eq!(OtlpEndpoint::parse("HTTP://c", true).unwrap().port(), 80);
    }

    /// Tracing: TC-045-02
    /// ACs: FR-045-AC-2
    #[test]
    fn tc_045_02_an_unusable_endpoint_is_refused_with_its_reason() {
        for (text, want) in [
            ("", EndpointError::Empty),
            ("https://collector:4318", EndpointError::Https),
            ("HTTPS://collector", EndpointError::Https),
            ("collector:4318", EndpointError::NotHttp),
            ("grpc://collector", EndpointError::NotHttp),
            ("http://user:pw@collector", EndpointError::Credentials),
            (
                "http://collector/v1?token=1",
                EndpointError::QueryOrFragment,
            ),
            ("http://collector#x", EndpointError::QueryOrFragment),
            ("http://coll ector", EndpointError::BadCharacter),
            ("http://collector\r\nX: y", EndpointError::BadCharacter),
            ("http://:4318", EndpointError::BadHost),
            ("http://[::1", EndpointError::BadHost),
            ("http://a:b:c", EndpointError::BadHost),
            ("http://collector:0", EndpointError::BadPort),
            ("http://collector:70000", EndpointError::BadPort),
            ("http://collector:abc", EndpointError::BadPort),
        ] {
            assert_eq!(OtlpEndpoint::parse(text, true), Err(want), "{text:?}");
        }
    }

    /// Tracing: TC-045-02
    /// ACs: FR-045-AC-2
    #[test]
    fn tc_045_02_headers_parse_decode_and_never_print_their_values() {
        let h = OtlpHeaders::parse("Authorization=Bearer%20s3cret, x-team=relay").unwrap();
        assert_eq!(
            h.pairs(),
            [
                ("Authorization".to_owned(), "Bearer s3cret".to_owned()),
                ("x-team".to_owned(), "relay".to_owned())
            ]
        );
        let shown = format!("{h:?}");
        assert!(
            !shown.contains("s3cret") && !shown.contains("relay"),
            "{shown}"
        );
        assert!(shown.contains("Authorization=<redacted>"), "{shown}");
        for bad in [
            "k=%+a",
            "k=%-1",
            "k=%1",
            "novalue",
            "=v",
            "a b=v",
            "Content-Type=text/plain",
            "host=x",
            "k=v%0d%0aX: y",
            "k=%zz",
            "k=v\nX: y",
        ] {
            let error = OtlpHeaders::parse(bad).unwrap_err();
            assert!(!error.contains("X: y"), "{error}");
            assert!(OtlpHeaders::parse(bad).is_err(), "{bad:?}");
        }
        assert!(OtlpHeaders::parse(&"k=v,".repeat(MAX_HEADERS + 1)).is_err());
    }

    /// Tracing: TC-045-02
    /// ACs: FR-045-AC-2
    #[test]
    fn tc_045_02_resource_attributes_skip_malformed_pairs_and_are_bounded() {
        let (kept, skipped) = parse_resource_attributes("a=1, b=x%20y,novalue,=v,,c=");
        assert_eq!(
            kept,
            [
                ("a".to_owned(), "1".to_owned()),
                ("b".to_owned(), "x y".to_owned()),
                ("c".to_owned(), String::new())
            ]
        );
        assert_eq!(skipped, 2);
        // A lax hex reader would turn `%+a` into a newline; it stays literal.
        assert_eq!(parse_resource_attributes("a=%+a").0[0].1, "%+a");
        let many: String = (0..100).map(|i| format!("k{i}=v,")).collect();
        assert_eq!(
            parse_resource_attributes(&many).0.len(),
            MAX_RESOURCE_ATTRIBUTES
        );
    }
}
