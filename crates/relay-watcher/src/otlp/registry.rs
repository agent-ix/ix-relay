//! The typed metric registry: what the watcher fills and the exporter encodes.
//!
//! An [`Instrument`] is a name, a unit, a description and a [`Kind`]; it is a
//! `const`, so the catalogue of what the watcher can emit is a list of statics
//! ([`super::series`]) and not strings scattered over call sites. A [`Registry`]
//! collects the points of one push: an instrument, an [`Attrs`] set and a value.
//! The registry is the only place values are checked, so a caller cannot emit a
//! histogram as a gauge or a point that JSON cannot hold.
//!
//! Every attribute *value* is untrusted (herdr and the registry hold text other
//! processes write), so [`Attrs::with`] neutralises control, bidirectional and
//! invisible characters ([`relay_core::text`]) and cuts to [`MAX_LABEL_CHARS`]
//! characters before it is stored. Attribute *keys* are `&'static str`: they come
//! from the catalogue, never from data.

use std::collections::HashSet;

/// The longest attribute value kept, in characters, after sanitising.
pub const MAX_LABEL_CHARS: usize = 128;

/// The most points one [`Registry`] holds; more are dropped and counted.
pub const MAX_POINTS: usize = 6_000;

/// What sort of series an instrument is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    /// A value that goes up and down, read as of the push.
    Gauge,
    /// A monotonic cumulative count since the watcher started.
    Sum,
    /// A cumulative histogram with these explicit bucket upper bounds
    /// (ascending; there is one more bucket than bounds, for the rest).
    Histogram {
        /// The bucket upper bounds.
        bounds: &'static [f64],
    },
}

/// One metric the watcher can emit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Instrument {
    /// The OTLP metric name.
    pub name: &'static str,
    /// The UCUM unit (`"1"`, `"s"`, `"{message}"`).
    pub unit: &'static str,
    /// One sentence for a person reading the collector.
    pub description: &'static str,
    /// Gauge, sum or histogram.
    pub kind: Kind,
}

/// `raw` as an attribute value: unsafe characters replaced (newlines and tabs
/// too, so a value is one line) and the length bounded. JSON escaping is the
/// encoder's job, and runs after this so nothing it adds is cut in half.
#[must_use]
pub fn label_value(raw: &str) -> String {
    relay_core::text::sanitise_line(raw)
        .chars()
        .take(MAX_LABEL_CHARS)
        .collect()
}

/// An attribute set: catalogue keys with sanitised string values, in the order
/// they were added.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Attrs(Vec<(&'static str, String)>);

impl Attrs {
    /// No attributes.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `key` with `raw` made safe by [`label_value`].
    #[must_use]
    pub fn with(mut self, key: &'static str, raw: &str) -> Self {
        self.0.push((key, label_value(raw)));
        self
    }

    /// The pairs, in order.
    #[must_use]
    pub fn pairs(&self) -> &[(&'static str, String)] {
        &self.0
    }
}

/// A cumulative histogram: what a caller records into and the registry emits.
#[derive(Debug, Clone, PartialEq)]
pub struct HistogramData {
    bounds: &'static [f64],
    /// One count per bound, and a last for values above every bound.
    buckets: Vec<u64>,
    count: u64,
    sum: f64,
}

impl HistogramData {
    /// An empty histogram over `bounds`.
    #[must_use]
    pub fn new(bounds: &'static [f64]) -> Self {
        Self {
            bounds,
            buckets: vec![0; bounds.len() + 1],
            count: 0,
            sum: 0.0,
        }
    }

    /// Records one observation. A value that is not finite is ignored.
    pub fn record(&mut self, value: f64) {
        if !value.is_finite() {
            return;
        }
        let index = self
            .bounds
            .iter()
            .position(|bound| value <= *bound)
            .unwrap_or(self.bounds.len());
        if let Some(bucket) = self.buckets.get_mut(index) {
            *bucket = bucket.saturating_add(1);
        }
        self.count = self.count.saturating_add(1);
        self.sum += value;
    }

    /// How many observations.
    #[must_use]
    pub fn count(&self) -> u64 {
        self.count
    }

    /// The sum of the observations.
    #[must_use]
    pub fn sum(&self) -> f64 {
        self.sum
    }

    /// The bucket upper bounds.
    #[must_use]
    pub fn bounds(&self) -> &'static [f64] {
        self.bounds
    }

    /// The per-bucket counts (one more than the bounds).
    #[must_use]
    pub fn buckets(&self) -> &[u64] {
        &self.buckets
    }
}

/// The value of one point.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// A whole number.
    Int(i64),
    /// A histogram.
    Histogram(HistogramData),
}

/// One series at one push.
#[derive(Debug, Clone, PartialEq)]
pub struct Point {
    /// Which metric.
    pub instrument: &'static Instrument,
    /// Its attributes.
    pub attrs: Attrs,
    /// Its value.
    pub value: Value,
}

/// The points of one push and the times they describe.
#[derive(Debug, Clone, PartialEq)]
pub struct Registry {
    /// When the values were read, Unix nanoseconds.
    pub time_unix_nano: u64,
    /// When the cumulative series began (the watcher's start), Unix nanoseconds.
    pub start_unix_nano: u64,
    points: Vec<Point>,
    seen: HashSet<(&'static str, Attrs)>,
    /// Points not kept: past [`MAX_POINTS`], a duplicate series, a value the
    /// instrument's kind does not take, or one JSON cannot hold; plus what the
    /// encoder later leaves out and what a caller folded (see
    /// [`Registry::note_dropped`]).
    dropped: u64,
}

impl Registry {
    /// An empty registry for a push at `time_unix_nano` of series that began at
    /// `start_unix_nano`.
    #[must_use]
    pub fn new(time_unix_nano: u64, start_unix_nano: u64) -> Self {
        Self {
            time_unix_nano,
            start_unix_nano,
            points: Vec::new(),
            seen: HashSet::new(),
            dropped: 0,
        }
    }

    /// The points kept, in the order they were added.
    #[must_use]
    pub fn points(&self) -> &[Point] {
        &self.points
    }

    /// How many points were not kept or were folded away.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Adds to the dropped count: points a caller folded into an aggregate or the
    /// encoder cut.
    pub fn note_dropped(&mut self, n: u64) {
        self.dropped = self.dropped.saturating_add(n);
    }

    /// Adds a point. Returns whether it was kept. A second point with the same
    /// instrument and attributes is a duplicate series: for a sum the values are
    /// added, otherwise the first is kept and the second counted as dropped.
    pub fn add(&mut self, instrument: &'static Instrument, attrs: Attrs, value: Value) -> bool {
        let fits = match (&instrument.kind, &value) {
            (Kind::Gauge, Value::Int(_)) => true,
            (Kind::Sum, Value::Int(v)) => *v >= 0,
            (Kind::Histogram { bounds }, Value::Histogram(h)) => *bounds == h.bounds,
            _ => false,
        };
        if !fits {
            self.dropped = self.dropped.saturating_add(1);
            return false;
        }
        let key = (instrument.name, attrs);
        if self.seen.contains(&key) {
            if let (Kind::Sum, Value::Int(add)) = (&instrument.kind, &value)
                && let Some(existing) = self
                    .points
                    .iter_mut()
                    .find(|p| p.instrument.name == key.0 && p.attrs == key.1)
                && let Value::Int(v) = &mut existing.value
            {
                *v = v.saturating_add(*add);
                return true;
            }
            self.dropped = self.dropped.saturating_add(1);
            return false;
        }
        if self.points.len() >= MAX_POINTS {
            self.dropped = self.dropped.saturating_add(1);
            return false;
        }
        let (name, attrs) = key;
        self.seen.insert((name, attrs.clone()));
        self.points.push(Point {
            instrument,
            attrs,
            value,
        });
        true
    }

    /// Adds a gauge point holding a whole number.
    pub fn gauge(&mut self, instrument: &'static Instrument, attrs: Attrs, value: i64) -> bool {
        self.add(instrument, attrs, Value::Int(value))
    }

    /// Adds a cumulative count.
    pub fn sum(&mut self, instrument: &'static Instrument, attrs: Attrs, value: u64) -> bool {
        self.add(
            instrument,
            attrs,
            Value::Int(i64::try_from(value).unwrap_or(i64::MAX)),
        )
    }

    /// Adds a cumulative histogram.
    pub fn histogram(
        &mut self,
        instrument: &'static Instrument,
        attrs: Attrs,
        data: &HistogramData,
    ) -> bool {
        self.add(instrument, attrs, Value::Histogram(data.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static G: Instrument = Instrument {
        name: "t.gauge",
        unit: "1",
        description: "g",
        kind: Kind::Gauge,
    };
    static S: Instrument = Instrument {
        name: "t.sum",
        unit: "1",
        description: "s",
        kind: Kind::Sum,
    };
    const BOUNDS: &[f64] = &[1.0, 5.0];
    static H: Instrument = Instrument {
        name: "t.hist",
        unit: "s",
        description: "h",
        kind: Kind::Histogram { bounds: BOUNDS },
    };

    /// Tracing: TC-045-09
    /// ACs: FR-045-AC-9
    #[test]
    fn tc_045_09_attribute_values_are_neutralised_and_bounded() {
        let a = Attrs::new()
            .with("k", "a\"b\\c\nd\u{1b}[31m\u{202e}e\u{200b}f")
            .with("long", &"W".repeat(10_240));
        let p = a.pairs();
        assert_eq!(p[0].1, "a\"b\\c\u{FFFD}d\u{FFFD}[31m\u{FFFD}e\u{FFFD}f");
        assert_eq!(p[1].1.chars().count(), MAX_LABEL_CHARS);
        let wide = Attrs::new().with("k", &"\u{1F600}".repeat(1000));
        assert_eq!(wide.pairs()[0].1.chars().count(), MAX_LABEL_CHARS);
    }

    /// Tracing: TC-045-09
    /// ACs: FR-045-AC-9
    #[test]
    fn tc_045_09_the_registry_refuses_what_its_kind_cannot_hold() {
        let mut r = Registry::new(2, 1);
        assert!(r.gauge(&G, Attrs::new(), 1));
        assert!(!r.add(&S, Attrs::new(), Value::Int(-1)));
        assert!(!r.add(
            &G,
            Attrs::new().with("a", "d"),
            Value::Histogram(HistogramData::new(BOUNDS))
        ));
        assert!(!r.add(&H, Attrs::new(), Value::Int(1)));
        assert!(!r.histogram(&H, Attrs::new(), &HistogramData::new(&[9.0])));
        assert!(r.histogram(&H, Attrs::new(), &HistogramData::new(BOUNDS)));
        assert_eq!(r.points().len(), 2);
        assert_eq!(r.dropped(), 4);
    }

    /// Tracing: TC-045-09
    /// ACs: FR-045-AC-9
    #[test]
    fn tc_045_09_a_duplicate_series_is_added_for_a_sum_and_dropped_otherwise() {
        let mut r = Registry::new(2, 1);
        assert!(r.gauge(&G, Attrs::new().with("a", "x"), 1));
        assert!(!r.gauge(&G, Attrs::new().with("a", "x"), 9), "first wins");
        assert!(r.sum(&S, Attrs::new().with("a", "x"), 2));
        assert!(r.sum(&S, Attrs::new().with("a", "x"), 3));
        assert_eq!(r.points()[0].value, Value::Int(1));
        assert_eq!(r.points()[1].value, Value::Int(5));
        assert_eq!(r.dropped(), 1);
        // Two raw values that sanitise to the same text are one series.
        let mut r = Registry::new(2, 1);
        assert!(r.gauge(&G, Attrs::new().with("a", "x\ny"), 1));
        assert!(!r.gauge(&G, Attrs::new().with("a", "x\ry"), 1));
    }

    /// Tracing: TC-045-09
    /// ACs: FR-045-AC-9
    #[test]
    fn tc_045_09_the_registry_is_bounded_in_points() {
        let mut r = Registry::new(2, 1);
        for n in 0..MAX_POINTS + 10 {
            r.gauge(&G, Attrs::new().with("n", &n.to_string()), 1);
        }
        assert_eq!(r.points().len(), MAX_POINTS);
        assert_eq!(r.dropped(), 10);
    }

    #[test]
    fn a_histogram_puts_each_value_in_the_first_bucket_that_holds_it() {
        let mut h = HistogramData::new(BOUNDS);
        for v in [0.5, 1.0, 1.5, 5.0, 5.1, f64::NAN] {
            h.record(v);
        }
        assert_eq!(h.buckets(), [2, 2, 1]);
        assert_eq!(h.count(), 5);
        assert!((h.sum() - 13.1).abs() < 1e-9);
    }
}
