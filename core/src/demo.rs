//! محاكاة تحميل لإثبات مسار التقدم: النواة ← جسر Tauri ← الواجهة.
//! تُحذف عند بناء محرك التحميل الحقيقي في M1.

use std::{thread, time::Duration};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    Started { total: u64 },
    Advanced { downloaded: u64 },
    Finished,
}

/// يحاكي تحميل `total` بايت على `steps` خطوات، وينتظر `delay` قبل كل خطوة.
/// إن كانت `steps` صفراً تُعامل كخطوة واحدة.
pub fn run(total: u64, steps: u32, delay: Duration, mut on_progress: impl FnMut(Progress)) {
    let steps = steps.max(1);
    on_progress(Progress::Started { total });
    for i in 1..=steps {
        thread::sleep(delay);
        let downloaded = (u128::from(total) * u128::from(i) / u128::from(steps)) as u64;
        on_progress(Progress::Advanced { downloaded });
    }
    on_progress(Progress::Finished);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(total: u64, steps: u32) -> Vec<Progress> {
        let mut events = Vec::new();
        run(total, steps, Duration::ZERO, |e| events.push(e));
        events
    }

    #[test]
    fn starts_advances_to_total_then_finishes() {
        let events = collect(1000, 4);
        assert_eq!(events.len(), 6);
        assert_eq!(events.first(), Some(&Progress::Started { total: 1000 }));
        assert_eq!(events[4], Progress::Advanced { downloaded: 1000 });
        assert_eq!(events.last(), Some(&Progress::Finished));
    }

    #[test]
    fn progress_never_goes_backwards() {
        let mut last = 0;
        for event in collect(999, 7) {
            if let Progress::Advanced { downloaded } = event {
                assert!(downloaded >= last);
                last = downloaded;
            }
        }
        assert_eq!(last, 999);
    }

    #[test]
    fn zero_steps_is_treated_as_one() {
        let events = collect(10, 0);
        assert_eq!(events.len(), 3);
        assert_eq!(events[1], Progress::Advanced { downloaded: 10 });
    }

    #[test]
    fn huge_totals_do_not_overflow() {
        let events = collect(u64::MAX, 3);
        assert_eq!(
            events[3],
            Progress::Advanced {
                downloaded: u64::MAX
            }
        );
    }
}
