//! Turning core's per-chunk progress into something a UI can receive.
//!
//! `download()` fires its callback for every chunk and says in its own docs
//! that throttling to 100–250 ms belongs to the channel layer (AGENTS.md §5.7).
//! That is this file, and nothing else.

use std::time::Instant;

use crate::dto::InstallEventDto;
use mm_core::downloader::ItemProgress;

/// The smallest gap AGENTS.md §5.7 allows, and the largest that still keeps a
/// moving progress bar smooth.
const MIN_INTERVAL_MS: u64 = 100;

/// Sends the first event and then at most one per [`MIN_INTERVAL_MS`].
pub struct ProgressThrottle {
    last_sent: Option<Instant>,
}

impl ProgressThrottle {
    pub fn new() -> Self {
        Self { last_sent: None }
    }

    /// Whether this event should cross the channel now. `terminal` events always
    /// should: they are how the UI learns the batch ended at all.
    pub fn allows(&mut self, terminal: bool, now: Instant) -> bool {
        if terminal {
            self.last_sent = Some(now);
            return true;
        }
        match self.last_sent {
            None => {
                self.last_sent = Some(now);
                true
            }
            Some(last) if now.duration_since(last).as_millis() as u64 >= MIN_INTERVAL_MS => {
                self.last_sent = Some(now);
                true
            }
            Some(_) => false,
        }
    }
}

impl Default for ProgressThrottle {
    fn default() -> Self {
        Self::new()
    }
}

/// Maps one core event onto the wire.
///
/// `phase` is a key, not a word: core says what is true, the UI says how to put
/// it. A phase string from a future core would surface as a missing translation
/// rather than as English leaking into an Arabic screen.
pub fn to_dto(item_id: &str, p: &ItemProgress) -> InstallEventDto {
    let (phase, done, total, path, message) = match p {
        ItemProgress::Started { done, total } => {
            ("install.phase.downloading", Some(*done), *total, None, None)
        }
        ItemProgress::Verifying => ("install.phase.verifying", None, None, None, None),
        ItemProgress::Landed { path } => (
            "install.phase.landed",
            None,
            None,
            Some(path.to_string_lossy().into_owned()),
            None,
        ),
        ItemProgress::Skipped { path } => (
            "install.phase.skipped",
            None,
            None,
            Some(path.to_string_lossy().into_owned()),
            None,
        ),
        ItemProgress::Failed { message } => (
            "install.phase.failed",
            None,
            None,
            None,
            Some(message.clone()),
        ),
        ItemProgress::Cancelled => ("install.phase.cancelled", None, None, None, None),
    };
    InstallEventDto {
        item_id: item_id.to_string(),
        phase: phase.to_string(),
        done,
        total,
        path,
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn spec_f005_throttle_sends_the_first_event_and_then_holds() {
        let mut t = ProgressThrottle::new();
        let now = Instant::now();
        assert!(t.allows(false, now), "the first event must cross");
        assert!(!t.allows(false, now + Duration::from_millis(10)));
        assert!(!t.allows(false, now + Duration::from_millis(99)));
        assert!(
            t.allows(false, now + Duration::from_millis(100)),
            "100ms is the floor"
        );
        assert!(!t.allows(false, now + Duration::from_millis(120)));
    }

    #[test]
    fn spec_f005_throttle_never_drops_a_terminal_event() {
        let mut t = ProgressThrottle::new();
        let now = Instant::now();
        assert!(t.allows(false, now));
        // Well inside the interval: a throttle that swallowed this would leave
        // the bar spinning with no way to learn the batch ended.
        assert!(t.allows(true, now + Duration::from_millis(1)));
        assert!(t.allows(true, now + Duration::from_millis(2)));
    }

    #[test]
    fn spec_f005_every_core_event_maps_to_a_key_not_a_display_string() {
        let events = [
            ItemProgress::Started {
                done: 5,
                total: Some(10),
            },
            ItemProgress::Verifying,
            ItemProgress::Landed {
                path: std::path::PathBuf::from("/a.jar"),
            },
            ItemProgress::Skipped {
                path: std::path::PathBuf::from("/b.jar"),
            },
            ItemProgress::Failed {
                message: "boom".into(),
            },
            ItemProgress::Cancelled,
        ];
        for e in events {
            let d = to_dto("i1", &e);
            assert!(
                d.phase.starts_with("install.phase."),
                "not a key: {}",
                d.phase
            );
        }
    }
}
