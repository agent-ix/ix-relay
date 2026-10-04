//! Pure relay directory lifecycle. Registration and watcher activity are not
//! substitutes for an unexpired, complete topology observation.

use std::collections::BTreeMap;

/// Monotonic publication fence assigned by the relay to an owning observer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Fence {
    /// Observer ownership generation.
    pub epoch: u64,
    /// Observation number within that generation.
    pub revision: u64,
}

/// Address ownership proven by one complete local topology observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Binding<S> {
    /// Exactly one verified root session owns this address.
    Unique(S),
    /// Multiple live sessions claim this address; routing is prohibited.
    Ambiguous,
}

/// A machine's atomic directory publication; times use the relay clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot<S> {
    /// Publication order, independent of the publishing machine's clock.
    pub fence: Fence,
    /// Relay-clock lease deadline, exclusive.
    pub expires_at: u64,
    /// Complete canonical address ownership map; omitted addresses are absent.
    pub bindings: BTreeMap<String, Binding<S>>,
}

/// Result of resolving an address without any Herdr or SSH operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution<'a, S> {
    /// No successful observation has been published.
    Unknown,
    /// The last observation's lease expired; request a local watcher refresh.
    Expired,
    /// A complete fresh observation contains no owner for this address.
    Absent,
    /// A fresh observation contains conflicting owners.
    Ambiguous,
    /// A uniquely owned address, fenced for atomic enqueue validation.
    Unique {
        /// Genuine session owner.
        session: &'a S,
        /// Enqueue must compare this fence and recheck the lease.
        fence: Fence,
    },
}

/// Accepted topology state. Failed observations do not mutate this state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Directory<S> {
    current: Option<Snapshot<S>>,
    observing: Option<Fence>,
}

impl<S> Default for Directory<S> {
    fn default() -> Self {
        Self {
            current: None,
            observing: None,
        }
    }
}

impl<S> Directory<S> {
    /// Start an ordered observation before reading topology. A newer start
    /// fences delayed publishers without invalidating the accepted snapshot.
    pub fn begin_observation(&mut self, fence: Fence) -> bool {
        if self.observing.is_some_and(|previous| fence <= previous) {
            return false;
        }
        self.observing = Some(fence);
        true
    }

    /// Accepts a complete observation only if it is newer and already fresh.
    /// Replays cannot renew a lease; callers must allocate fences at the relay.
    pub fn publish(&mut self, snapshot: Snapshot<S>, now: u64) -> bool {
        if self.observing != Some(snapshot.fence)
            || snapshot.expires_at <= now
            || self
                .current
                .as_ref()
                .is_some_and(|old| snapshot.fence <= old.fence)
        {
            return false;
        }
        self.current = Some(snapshot);
        true
    }

    /// Resolves solely from the accepted directory state.
    pub fn resolve(&self, address: &str, now: u64) -> Resolution<'_, S> {
        let Some(snapshot) = &self.current else {
            return Resolution::Unknown;
        };
        if now >= snapshot.expires_at {
            return Resolution::Expired;
        }
        match snapshot.bindings.get(address) {
            None => Resolution::Absent,
            Some(Binding::Ambiguous) => Resolution::Ambiguous,
            Some(Binding::Unique(session)) => Resolution::Unique {
                session,
                fence: snapshot.fence,
            },
        }
    }

    /// The enqueue linearization guard: neither an old fence nor an expired
    /// lease authorizes routing, even if resolution previously succeeded.
    pub fn permits_enqueue(&self, address: &str, fence: Fence, now: u64) -> bool {
        matches!(self.resolve(address, now), Resolution::Unique { fence: current, .. } if current == fence)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(epoch: u64, revision: u64, owner: Option<Binding<u8>>) -> Snapshot<u8> {
        Snapshot {
            fence: Fence { epoch, revision },
            expires_at: 10,
            bindings: owner
                .into_iter()
                .map(|owner| ("terra:QSL:plan".into(), owner))
                .collect(),
        }
    }

    #[test]
    fn expiry_and_replacement_fence_the_enqueue_race() {
        let mut directory = Directory::default();
        directory.begin_observation(Fence {
            epoch: 1,
            revision: 1,
        });
        assert!(directory.publish(snapshot(1, 1, Some(Binding::Unique(1))), 0));
        let old = Fence {
            epoch: 1,
            revision: 1,
        };
        assert!(directory.permits_enqueue("terra:QSL:plan", old, 9));
        assert!(!directory.permits_enqueue("terra:QSL:plan", old, 10));
        directory.begin_observation(Fence {
            epoch: 1,
            revision: 2,
        });
        assert!(directory.publish(snapshot(1, 2, Some(Binding::Unique(2))), 1));
        assert!(!directory.permits_enqueue("terra:QSL:plan", old, 1));
        assert!(!directory.publish(snapshot(1, 1, Some(Binding::Unique(1))), 1));
    }

    #[test]
    fn empty_and_ambiguous_observations_never_authorize_enqueue() {
        let mut directory = Directory::default();
        directory.begin_observation(Fence {
            epoch: 1,
            revision: 1,
        });
        assert!(directory.publish(snapshot(1, 1, Some(Binding::Ambiguous)), 0));
        assert_eq!(
            directory.resolve("terra:QSL:plan", 0),
            Resolution::Ambiguous
        );
        directory.begin_observation(Fence {
            epoch: 1,
            revision: 2,
        });
        assert!(directory.publish(snapshot(1, 2, None), 0));
        assert_eq!(directory.resolve("terra:QSL:plan", 0), Resolution::Absent);
    }

    #[test]
    fn bounded_event_sequences_preserve_publication_and_enqueue_invariants() {
        // Exhaustively explore four-event sequences over competing epochs,
        // revisions, owners, closure, ambiguity, clock advance, and failed reads.
        fn explore(directory: Directory<u8>, now: u64, depth: u8) {
            if depth == 0 {
                return;
            }
            for event in 0..14 {
                let mut next = directory.clone();
                let next_now = if event == 12 { now + 5 } else { now };
                if event < 12 {
                    let epoch = u64::from(event / 6 + 1);
                    let revision = u64::from(event % 3 + 1);
                    let owner = match event % 4 {
                        0 => None,
                        1 => Some(Binding::Ambiguous),
                        n => Some(Binding::Unique(n)),
                    };
                    let candidate = snapshot(epoch, revision, owner);
                    next.begin_observation(candidate.fence);
                    let before = next.clone();
                    let accepted = next.publish(candidate.clone(), now);
                    if accepted {
                        assert_eq!(next.current.as_ref(), Some(&candidate));
                        assert!(candidate.expires_at > now);
                        assert!(
                            before
                                .current
                                .as_ref()
                                .is_none_or(|old| old.fence < candidate.fence)
                        );
                    } else {
                        assert_eq!(next, before);
                    }
                }
                // Event 13 is a failed/incomplete read: no publication.
                for epoch in 1..=2 {
                    for revision in 1..=3 {
                        let fence = Fence { epoch, revision };
                        if next.permits_enqueue("terra:QSL:plan", fence, next_now) {
                            let current = next.current.as_ref().unwrap();
                            assert!(next_now < current.expires_at);
                            assert_eq!(current.fence, fence);
                            assert!(matches!(
                                current.bindings.get("terra:QSL:plan"),
                                Some(Binding::Unique(_))
                            ));
                        }
                    }
                }
                explore(next, next_now, depth - 1);
            }
        }
        explore(Directory::default(), 0, 4);
    }
}
