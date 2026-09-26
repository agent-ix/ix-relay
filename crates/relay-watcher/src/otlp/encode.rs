//! Turns a [`Registry`] into an OTLP `ExportMetricsServiceRequest` in the JSON
//! encoding of OTLP/HTTP (`Content-Type: application/json`).
//!
//! JSON, not protobuf, so no protobuf dependency is needed. The mapping follows
//! the protobuf JSON rules OTLP specifies: field names in lowerCamelCase, 64-bit
//! integers and nanosecond timestamps as decimal strings, enums as integers.
//! The request is a tree of `Serialize` structs, never a hand-built
//! `serde_json::Value`, so the compiler checks that each point carries its
//! required members and the JSON escaping of every string is `serde_json`'s.
//!
//! The request is bounded to [`MAX_REQUEST_BYTES`]: points past it are left out
//! and counted, never sent.

use std::collections::BTreeMap;

use serde::Serialize;

use super::registry::{Instrument, Kind, Point, Registry, Value, label_value};

/// The most bytes one request body takes. A collector's own default limit is
/// several MiB; this keeps a request well inside it.
pub const MAX_REQUEST_BYTES: usize = 4 * 1024 * 1024;

/// What the fixed structure around the points takes, counted generously.
const OVERHEAD_BYTES: usize = 16 * 1024;

/// `AGGREGATION_TEMPORALITY_CUMULATIVE`.
const CUMULATIVE: u8 = 2;

/// The instrumentation scope every metric is reported under.
pub const SCOPE_NAME: &str = "ix-relay-watcher";

/// The resource a push describes: who is sending. Attribute keys and values are
/// made safe like any other label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resource {
    attrs: Vec<(String, String)>,
    version: String,
}

impl Resource {
    /// The resource of a watcher: `service.name`, `host.name` and
    /// `service.version`, then `extra` (`OTEL_RESOURCE_ATTRIBUTES`) for any key
    /// not already set. `service_name` wins over an extra `service.name`, which
    /// wins over `default_service`.
    #[must_use]
    pub fn new(
        default_service: &str,
        service_name: Option<&str>,
        host: &str,
        version: &str,
        extra: &[(String, String)],
    ) -> Self {
        let from_extra = extra
            .iter()
            .find(|(k, _)| k == "service.name")
            .map(|(_, v)| v.as_str());
        let service = service_name.or(from_extra).unwrap_or(default_service);
        let mut attrs = vec![
            ("service.name".to_owned(), label_value(service)),
            ("host.name".to_owned(), label_value(host)),
            ("service.version".to_owned(), label_value(version)),
        ];
        for (key, value) in extra {
            let key = label_value(key);
            if !key.is_empty() && !attrs.iter().any(|(k, _)| *k == key) {
                attrs.push((key, label_value(value)));
            }
        }
        Self {
            attrs,
            version: version.to_owned(),
        }
    }
}

/// An encoded request and what it left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Encoded {
    /// The JSON body.
    pub body: Vec<u8>,
    /// Points left out to stay inside [`MAX_REQUEST_BYTES`].
    pub cut: u64,
    /// How many points the body carries.
    pub points: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Request<'a> {
    resource_metrics: [ResourceMetrics<'a>; 1],
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ResourceMetrics<'a> {
    resource: ResourceJson<'a>,
    scope_metrics: [ScopeMetrics<'a>; 1],
}

#[derive(Serialize)]
struct ResourceJson<'a> {
    attributes: Vec<KeyValue<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScopeMetrics<'a> {
    scope: Scope<'a>,
    metrics: Vec<Metric<'a>>,
}

#[derive(Serialize)]
struct Scope<'a> {
    name: &'static str,
    version: &'a str,
}

#[derive(Serialize)]
struct KeyValue<'a> {
    key: &'a str,
    value: AnyValue<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AnyValue<'a> {
    string_value: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Metric<'a> {
    name: &'static str,
    description: &'static str,
    unit: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    gauge: Option<Gauge<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sum: Option<Sum<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    histogram: Option<HistogramJson<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Gauge<'a> {
    data_points: Vec<NumberPoint<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Sum<'a> {
    data_points: Vec<NumberPoint<'a>>,
    aggregation_temporality: u8,
    is_monotonic: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HistogramJson<'a> {
    data_points: Vec<HistogramPoint<'a>>,
    aggregation_temporality: u8,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NumberPoint<'a> {
    attributes: Vec<KeyValue<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    start_time_unix_nano: Option<String>,
    time_unix_nano: String,
    as_int: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HistogramPoint<'a> {
    attributes: Vec<KeyValue<'a>>,
    start_time_unix_nano: String,
    time_unix_nano: String,
    count: String,
    sum: f64,
    bucket_counts: Vec<String>,
    explicit_bounds: &'a [f64],
}

fn attributes(point: &Point) -> Vec<KeyValue<'_>> {
    point
        .attrs
        .pairs()
        .iter()
        .map(|(key, value)| KeyValue {
            key,
            value: AnyValue {
                string_value: value,
            },
        })
        .collect()
}

fn number_point<'a>(
    registry: &Registry,
    point: &'a Point,
    cumulative: bool,
) -> Option<NumberPoint<'a>> {
    let as_int = match &point.value {
        Value::Int(v) => v.to_string(),
        Value::Histogram(_) => return None,
    };
    Some(NumberPoint {
        attributes: attributes(point),
        start_time_unix_nano: cumulative.then(|| registry.start_unix_nano.to_string()),
        time_unix_nano: registry.time_unix_nano.to_string(),
        as_int,
    })
}

fn histogram_point<'a>(registry: &Registry, point: &'a Point) -> Option<HistogramPoint<'a>> {
    let Value::Histogram(h) = &point.value else {
        return None;
    };
    Some(HistogramPoint {
        attributes: attributes(point),
        start_time_unix_nano: registry.start_unix_nano.to_string(),
        time_unix_nano: registry.time_unix_nano.to_string(),
        count: h.count().to_string(),
        sum: h.sum(),
        bucket_counts: h.buckets().iter().map(u64::to_string).collect(),
        explicit_bounds: h.bounds(),
    })
}

/// The points of one instrument, kept in order.
enum Group<'a> {
    Number(Vec<NumberPoint<'a>>),
    Histogram(Vec<HistogramPoint<'a>>),
}

/// Encodes `registry` for `resource`. Points are kept in the order they were
/// added until [`MAX_REQUEST_BYTES`] would be passed; the rest are left out and
/// counted in [`Encoded::cut`].
///
/// # Errors
///
/// A [`serde_json::Error`] if serialising fails, which the typed tree cannot
/// cause for the values a [`Registry`] accepts.
pub fn encode(registry: &Registry, resource: &Resource) -> Result<Encoded, serde_json::Error> {
    let budget = MAX_REQUEST_BYTES.saturating_sub(OVERHEAD_BYTES);
    let mut used = 0usize;
    let mut cut = 0u64;
    let mut kept = 0u64;
    // Instruments in order of first appearance, each with its points.
    let mut order: Vec<&'static Instrument> = Vec::new();
    let mut groups: BTreeMap<&'static str, Group<'_>> = BTreeMap::new();
    for point in registry.points() {
        let instrument = point.instrument;
        let cumulative = matches!(instrument.kind, Kind::Sum);
        let size;
        let group = groups.entry(instrument.name).or_insert_with(|| {
            order.push(instrument);
            match instrument.kind {
                Kind::Histogram { .. } => Group::Histogram(Vec::new()),
                Kind::Gauge | Kind::Sum => Group::Number(Vec::new()),
            }
        });
        match group {
            Group::Number(points) => {
                let Some(p) = number_point(registry, point, cumulative) else {
                    cut += 1;
                    continue;
                };
                size = serde_json::to_vec(&p)?.len() + 1;
                if used + size > budget {
                    cut += 1;
                    continue;
                }
                points.push(p);
            }
            Group::Histogram(points) => {
                let Some(p) = histogram_point(registry, point) else {
                    cut += 1;
                    continue;
                };
                size = serde_json::to_vec(&p)?.len() + 1;
                if used + size > budget {
                    cut += 1;
                    continue;
                }
                points.push(p);
            }
        }
        used += size;
        kept += 1;
    }
    let mut metrics = Vec::new();
    for instrument in order {
        let Some(group) = groups.remove(instrument.name) else {
            continue;
        };
        let mut metric = Metric {
            name: instrument.name,
            description: instrument.description,
            unit: instrument.unit,
            gauge: None,
            sum: None,
            histogram: None,
        };
        match (group, instrument.kind) {
            (Group::Number(data_points), Kind::Gauge) if !data_points.is_empty() => {
                metric.gauge = Some(Gauge { data_points });
            }
            (Group::Number(data_points), Kind::Sum) if !data_points.is_empty() => {
                metric.sum = Some(Sum {
                    data_points,
                    aggregation_temporality: CUMULATIVE,
                    is_monotonic: true,
                });
            }
            (Group::Histogram(data_points), Kind::Histogram { .. }) if !data_points.is_empty() => {
                metric.histogram = Some(HistogramJson {
                    data_points,
                    aggregation_temporality: CUMULATIVE,
                });
            }
            _ => continue,
        }
        metrics.push(metric);
    }
    let request = Request {
        resource_metrics: [ResourceMetrics {
            resource: ResourceJson {
                attributes: resource
                    .attrs
                    .iter()
                    .map(|(key, value)| KeyValue {
                        key,
                        value: AnyValue {
                            string_value: value,
                        },
                    })
                    .collect(),
            },
            scope_metrics: [ScopeMetrics {
                scope: Scope {
                    name: SCOPE_NAME,
                    version: &resource.version,
                },
                metrics,
            }],
        }],
    };
    Ok(Encoded {
        body: serde_json::to_vec(&request)?,
        cut,
        points: kept,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::otlp::{
        registry::{Attrs, HistogramData, MAX_LABEL_CHARS},
        series::{MESSAGES, PANE_INFO, TICK_BOUNDS, TICK_DURATION},
    };

    fn resource() -> Resource {
        Resource::new(
            "ix-relay-watcher",
            None,
            "hub",
            "0.1.0",
            &[("team".into(), "relay".into())],
        )
    }

    /// The whole request for a fake registry, byte for byte, written out by hand
    /// from the OTLP JSON mapping (int64 and timestamps as strings, lowerCamelCase
    /// names, cumulative temporality 2, monotonic sums, bucket counts one more than
    /// the bounds), independent of the code's own field names.
    ///
    /// Tracing: TC-045-03
    /// ACs: FR-045-AC-3
    #[test]
    fn tc_045_03_the_request_for_a_fake_registry_is_byte_exact() {
        let mut r = Registry::new(2_000, 1_000);
        r.gauge(
            &PANE_INFO,
            Attrs::new()
                .with("machine", "hub")
                .with("pane", "w1:p1")
                .with("tab_label", "review"),
            1,
        );
        r.sum(&MESSAGES, Attrs::new().with("outcome", "delivered"), 3);
        let mut h = HistogramData::new(TICK_BOUNDS);
        h.record(0.02);
        r.histogram(&TICK_DURATION, Attrs::new(), &h);
        let expected = concat!(
            r#"{"resourceMetrics":[{"resource":{"attributes":["#,
            r#"{"key":"service.name","value":{"stringValue":"ix-relay-watcher"}},"#,
            r#"{"key":"host.name","value":{"stringValue":"hub"}},"#,
            r#"{"key":"service.version","value":{"stringValue":"0.1.0"}},"#,
            r#"{"key":"team","value":{"stringValue":"relay"}}]},"#,
            r#""scopeMetrics":[{"scope":{"name":"ix-relay-watcher","version":"0.1.0"},"metrics":["#,
            r#"{"name":"herdr.pane.info","description":"Names for the herdr panes on this machine (value 1); join on machine, pane.","unit":"1","#,
            r#""gauge":{"dataPoints":[{"attributes":["#,
            r#"{"key":"machine","value":{"stringValue":"hub"}},"#,
            r#"{"key":"pane","value":{"stringValue":"w1:p1"}},"#,
            r#"{"key":"tab_label","value":{"stringValue":"review"}}],"#,
            r#""timeUnixNano":"2000","asInt":"1"}]}},"#,
            r#"{"name":"ix.relay.messages","description":"Messages by outcome since this watcher started: delivered counts messages of batches this watcher delivered and saw confirmed, by typing into a pane or by a socket post confirmed within its wait (not hook deliveries, and not a held message approved later); requeued, redelivered and dead_lettered are what it put back or reaped; socket denials, postponements and deferrals, sent and denied are not counted.","unit":"{message}","#,
            r#""sum":{"dataPoints":[{"attributes":[{"key":"outcome","value":{"stringValue":"delivered"}}],"#,
            r#""startTimeUnixNano":"1000","timeUnixNano":"2000","asInt":"3"}],"#,
            r#""aggregationTemporality":2,"isMonotonic":true}},"#,
            r#"{"name":"ix.relay.watcher.tick.duration","description":"How long one watcher tick took, since the watcher started.","unit":"s","#,
            r#""histogram":{"dataPoints":[{"attributes":[],"startTimeUnixNano":"1000","timeUnixNano":"2000","#,
            r#""count":"1","sum":0.02,"bucketCounts":["0","1","0","0","0","0","0","0"],"#,
            r#""explicitBounds":[0.005,0.025,0.1,0.25,1.0,2.5,10.0]}],"aggregationTemporality":2}}"#,
            r#"]}]}]}"#,
        );
        let encoded = encode(&r, &resource()).unwrap();
        assert_eq!(String::from_utf8(encoded.body).unwrap(), expected);
        assert_eq!((encoded.points, encoded.cut), (3, 0));
    }

    /// `OTEL_SERVICE_NAME` beats a `service.name` in `OTEL_RESOURCE_ATTRIBUTES`,
    /// which beats the default; the resource keeps the three standard attributes
    /// first and ignores a duplicate key.
    ///
    /// Tracing: TC-045-03
    /// ACs: FR-045-AC-3
    #[test]
    fn tc_045_03_the_resource_takes_the_service_name_from_the_right_tier() {
        let extra = [
            ("service.name".to_owned(), "from-attrs".to_owned()),
            ("host.name".to_owned(), "spoofed".to_owned()),
            ("k".to_owned(), "v".to_owned()),
        ];
        let by = |name: Option<&str>| {
            Resource::new("default", name, "hub", "1", &extra)
                .attrs
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            by(Some("from-env")),
            [
                "service.name=from-env",
                "host.name=hub",
                "service.version=1",
                "k=v"
            ]
        );
        assert_eq!(by(None)[0], "service.name=from-attrs");
        assert_eq!(
            Resource::new("default", None, "hub", "1", &[]).attrs[0].1,
            "default"
        );
    }

    /// Hostile label text cannot break the JSON: quotes, backslashes, newlines,
    /// ESC, bidi and invisible characters, and a 10 KB value. The body still parses
    /// as JSON and every value is the neutralised, bounded one.
    ///
    /// Tracing: TC-045-03
    /// ACs: FR-045-AC-3
    #[test]
    fn tc_045_03_hostile_labels_leave_valid_json_with_neutralised_bounded_values() {
        let mut r = Registry::new(2, 1);
        r.gauge(
            &PANE_INFO,
            Attrs::new()
                .with(
                    "tab_label",
                    "a\"b\\c\nd\u{1b}[31m\u{202e}e\u{200b}f\",\"x\":\"y",
                )
                .with("workspace_label", &"W".repeat(10_240)),
            1,
        );
        let hostile = Resource::new(
            "s\"vc",
            Some("n\nm"),
            "h\u{1b}",
            "v",
            &[("k\"".into(), "v\u{202e}".into())],
        );
        let body = encode(&r, &hostile).unwrap().body;
        let text = String::from_utf8(body.clone()).unwrap();
        assert!(
            !text.contains(['\u{1b}', '\u{202e}', '\u{200b}', '\n']),
            "{text}"
        );
        let json: serde_json::Value = serde_json::from_slice(&body).expect("valid JSON");
        let point =
            &json["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0]["gauge"]["dataPoints"][0];
        let attr = |key: &str| {
            point["attributes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|a| a["key"] == key)
                .unwrap()["value"]["stringValue"]
                .as_str()
                .unwrap()
                .to_owned()
        };
        assert_eq!(
            attr("tab_label"),
            "a\"b\\c\u{FFFD}d\u{FFFD}[31m\u{FFFD}e\u{FFFD}f\",\"x\":\"y",
            "the quote and backslash survive as data, the rest is replaced"
        );
        assert_eq!(attr("workspace_label").chars().count(), MAX_LABEL_CHARS);
        assert_eq!(
            json["resourceMetrics"][0]["resource"]["attributes"][0]["value"]["stringValue"],
            "n\u{FFFD}m"
        );
        assert_eq!(
            point["attributes"].as_array().unwrap().len(),
            2,
            "no injected member"
        );
    }

    /// A request is bounded: past the byte budget points are left out and counted,
    /// and what is kept plus what is cut is everything.
    ///
    /// Tracing: TC-045-03
    /// ACs: FR-045-AC-3
    #[test]
    fn tc_045_03_a_request_over_the_byte_budget_leaves_out_points_and_counts_them() {
        let wide = "\u{1F600}".repeat(MAX_LABEL_CHARS);
        let mut r = Registry::new(2, 1);
        for n in 0..crate::otlp::registry::MAX_POINTS {
            let mut a = Attrs::new().with("n", &n.to_string());
            for key in [
                "machine",
                "pane",
                "tab",
                "workspace",
                "workspace_label",
                "tab_label",
                "session",
            ] {
                a = a.with(key, &wide);
            }
            r.gauge(&PANE_INFO, a, 1);
        }
        let e = encode(&r, &resource()).unwrap();
        assert!(e.body.len() <= MAX_REQUEST_BYTES, "{} bytes", e.body.len());
        assert!(e.cut > 0, "the budget was reached");
        assert_eq!(e.points + e.cut, r.points().len() as u64);
        serde_json::from_slice::<serde_json::Value>(&e.body).expect("still valid JSON");
    }
}
