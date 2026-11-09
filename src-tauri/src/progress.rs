//! Turning core's per-chunk progress into something a UI can receive.
//!
//! `download()` fires its callback for every chunk and says in its own docs
//! that throttling to 100–250 ms belongs to the channel layer (AGENTS.md §5.7).
//! That is this file, and nothing else.

use std::time::Instant;

use crate::dto::{InstallEventDto, PackEventDto};
use mm_core::downloader::ItemProgress;
use mm_core::modpack::exec::{phase as pack_phase, PackEvent};

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

/// Whether a modpack phase ends the run.
///
/// The throttle must never hold one of these back: `DONE` and `FAILED` are how
/// the screen learns that the apply stopped, and a progress view that goes
/// quiet is indistinguishable from a hang (AGENTS.md §5.7). `apply` in core
/// guarantees the last event is one of the two.
pub fn is_terminal_pack_phase(phase: &str) -> bool {
    phase == pack_phase::DONE || phase == pack_phase::FAILED
}

/// Whether a modpack phase is somebody's final answer about something.
///
/// A run ends on [`pack_phase::DONE`] or [`pack_phase::FAILED`]; a single file
/// settles on [`pack_phase::WRITTEN`] or [`pack_phase::SKIPPED`]. All four are
/// states, not progress: the screen keeps one row per destination and renders
/// its **last** event, so a throttled-away `written` would leave that row
/// standing at an intermediate phase forever. Downloads are the only events
/// worth dropping, and core already emits them once per chunk.
pub fn is_decisive_pack_phase(phase: &str) -> bool {
    is_terminal_pack_phase(phase) || phase == pack_phase::WRITTEN || phase == pack_phase::SKIPPED
}

/// Maps one core modpack event onto the wire.
///
/// A straight copy on purpose: core's `PackEvent` already carries the phase key,
/// the instance-relative path and the byte counters, and inventing a second
/// vocabulary here would be the drift the key convention exists to prevent.
pub fn pack_event_to_dto(e: &PackEvent) -> PackEventDto {
    PackEventDto {
        phase: e.phase.to_string(),
        safe_path: e.safe_path.clone(),
        done: e.done,
        total: e.total,
    }
}

/// The event a refused apply sends before returning its error.
///
/// Some refusals happen before core's executor exists (a hostile archive, an
/// unsafe plan, a failed extraction). Those runs never reach `apply`, so they
/// would otherwise end the channel silently while the promise rejected.
pub fn refused_pack_event() -> PackEventDto {
    PackEventDto {
        phase: pack_phase::FAILED.to_string(),
        safe_path: None,
        done: 0,
        total: None,
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

    /// The modpack channel reuses this throttle, so the properties that matter
    /// are pinned here as well: every pack phase is a key, only two of them end
    /// a run, and the throttle never drops a phase that is already a state.
    #[test]
    fn spec_f005_pack_phases_are_keys_and_only_two_end_the_run() {
        let phases = [
            pack_phase::COPYING,
            pack_phase::DOWNLOADING,
            pack_phase::VERIFYING,
            pack_phase::COMMITTING,
            pack_phase::WRITTEN,
            pack_phase::SKIPPED,
            pack_phase::DONE,
            pack_phase::FAILED,
        ];
        for phase in phases {
            assert!(phase.starts_with("pack.phase."), "not a key: {phase}");
            let dto = pack_event_to_dto(&PackEvent {
                phase,
                safe_path: None,
                done: 0,
                total: None,
            });
            assert_eq!(dto.phase, phase);
        }
        let terminal: Vec<&str> = phases
            .iter()
            .copied()
            .filter(|p| is_terminal_pack_phase(p))
            .collect();
        assert_eq!(terminal, vec![pack_phase::DONE, pack_phase::FAILED]);
        // A refused apply narrates itself too, or the progress view would sit
        // on the last progress event forever.
        assert!(is_terminal_pack_phase(&refused_pack_event().phase));

        // Dropping these would leave a row stuck on `downloading` even though
        // the file landed, so they must reach the channel undelayed.
        let decisive: Vec<&str> = phases
            .iter()
            .copied()
            .filter(|p| is_decisive_pack_phase(p))
            .collect();
        assert_eq!(
            decisive,
            vec![
                pack_phase::WRITTEN,
                pack_phase::SKIPPED,
                pack_phase::DONE,
                pack_phase::FAILED
            ]
        );
    }
}
