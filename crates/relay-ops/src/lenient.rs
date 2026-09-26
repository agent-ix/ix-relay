//! Per-entry tolerance for herdr's list output, where it is safe (AGE-2027).
//!
//! herdr owns the format of its listings. [`Lenient`] decodes each element on its
//! own, drops one that does not decode and **counts** it, so the caller can
//! report it (the watcher once per episode, `doctor` as "panes unchecked"). It is
//! for lists where a dropped entry only removes a row or a place update:
//! `agent list`, `workspace list` and `tab list` as `find` and the watcher's place
//! refresh read them. It is not for the lists name derivation reads
//! (`workspace list`, `tab list`, `pane list` in `naming`): those decide from
//! every sibling, and a dropped one changes the answer, so they stay strict.
//!
//! A non-empty array in which **every** element fails is not "some bad entries"
//! but a format change: it is an error, as a whole-list decode failure always was,
//! so the FR-036 check and `doctor` still see it.

use serde::{Deserialize, Deserializer, de::DeserializeOwned};

/// The elements of a herdr list that decoded, and how many did not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lenient<T> {
    /// The elements that decoded, in order.
    pub entries: Vec<T>,
    /// How many did not.
    pub dropped: usize,
}

impl<'de, T: DeserializeOwned> Deserialize<'de> for Lenient<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = Vec::<serde_json::Value>::deserialize(deserializer)?;
        let total = raw.len();
        let mut entries = Vec::with_capacity(total);
        let mut first_error = None;
        for value in raw {
            match serde_json::from_value(value) {
                Ok(entry) => entries.push(entry),
                Err(e) => {
                    first_error.get_or_insert(e);
                }
            }
        }
        if entries.is_empty()
            && let Some(e) = first_error
        {
            return Err(serde::de::Error::custom(format!(
                "none of the {total} list entries decoded: {e}"
            )));
        }
        Ok(Self {
            dropped: total - entries.len(),
            entries,
        })
    }
}

#[cfg(test)]
mod tests {
    use relay_core::PaneId;
    use serde::Deserialize;

    use super::*;

    #[derive(Deserialize)]
    struct Entry {
        pane_id: PaneId,
    }

    #[derive(Deserialize)]
    struct Listing {
        panes: Lenient<Entry>,
    }

    #[test]
    fn one_bad_entry_is_dropped_counted_and_the_others_survive() {
        let listing: Listing = serde_json::from_str(
            r#"{"panes":[{"pane_id":"w1:p1"},{"pane_id":""},{"pane_id":"w1:p\u200b2"},{"x":1},{"pane_id":"w1:p3"}]}"#,
        )
        .unwrap();
        let ids: Vec<&str> = listing
            .panes
            .entries
            .iter()
            .map(|e| e.pane_id.as_str())
            .collect();
        assert_eq!(ids, ["w1:p1", "w1:p3"]);
        assert_eq!(listing.panes.dropped, 3);
    }

    /// A format change hits every entry: that is a decode failure, not an empty list.
    #[test]
    fn a_list_in_which_every_entry_fails_is_an_error() {
        assert!(serde_json::from_str::<Listing>(r#"{"panes":[{"x":1},{"y":2}]}"#).is_err());
        let empty: Listing = serde_json::from_str(r#"{"panes":[]}"#).unwrap();
        assert_eq!((empty.panes.entries.len(), empty.panes.dropped), (0, 0));
    }

    #[test]
    fn a_listing_that_is_not_an_array_still_fails() {
        assert!(serde_json::from_str::<Listing>(r#"{"panes":{"pane_id":"w1:p1"}}"#).is_err());
    }
}
