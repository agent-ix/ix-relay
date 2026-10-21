//! `--since` values like `30s`, `15m`, `1h`, `2d`.

use std::{str::FromStr, time::Duration};

/// A look-back window for `agent-msg log`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Since(pub Duration);

impl FromStr for Since {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let bad = || format!("{s:?}: expected a number and a unit (s, m, h or d), e.g. 1h");
        let (unit_at, _) = s.char_indices().last().ok_or_else(bad)?;
        let (num, unit) = s.split_at(unit_at);
        let n: u64 = num.parse().map_err(|_| bad())?;
        let secs_per = match unit {
            "s" => 1,
            "m" => 60,
            "h" => 3_600,
            "d" => 86_400,
            _ => return Err(bad()),
        };
        n.checked_mul(secs_per)
            .map(|secs| Self(Duration::from_secs(secs)))
            .ok_or_else(bad)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trace: FR-057-AC-6
    #[test]
    fn parses_each_unit() {
        for (input, secs) in [("30s", 30), ("15m", 900), ("1h", 3_600), ("2d", 172_800)] {
            assert_eq!(
                input.parse::<Since>().unwrap(),
                Since(Duration::from_secs(secs))
            );
        }
    }

    /// Trace: FR-057-AC-6
    #[test]
    fn rejects_missing_unit_number_or_overflow() {
        for bad in [
            "",
            "h",
            "10",
            " 1h",
            "1 h",
            "1h ",
            "1w",
            "-1h",
            "1.5h",
            "1é",
            "99999999999999999999d",
        ] {
            assert!(bad.parse::<Since>().is_err(), "{bad:?} was accepted");
        }
    }
}
