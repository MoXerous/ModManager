//! Retry classification and delays (`F-039`).
//!
//! Mirrors `_raise_retryable_status` / `retry_delay_seconds`:
//! retryable = transport failure, `429`, `500..=599`; never anything else.
//! Delay = numeric `Retry-After`, else HTTP-date, else `base * 2^(n-1)` —
//! every branch clamped to `[0, max]`. Deterministic (no jitter): the old app
//! has none, and `foundations.md` §4.5's jitter wish stays open.

use reqwest::header::{HeaderMap, RETRY_AFTER};

/// Spec defaults: `backoff_base_seconds = 1.0`, `max_backoff_seconds = 30.0`,
/// `max_attempts = 1` (production wires 2; see `downloads.md` §6-3).
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub base_secs: f64,
    pub max_secs: f64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 1,
            base_secs: 1.0,
            max_secs: 30.0,
        }
    }
}

/// Retryable: transport failure (`None`), `429`, `500..=599`. Nothing else.
pub fn should_retry(status: Option<u16>) -> bool {
    match status {
        None => true,
        Some(429) => true,
        Some(s) if (500..=599).contains(&s) => true,
        _ => false,
    }
}

/// Delay in seconds: numeric `Retry-After`, else HTTP-date, else
/// `base * 2^(attempt-1)` — every branch clamped to `[0, max]`.
pub fn retry_delay_secs(attempt: u32, headers: &HeaderMap, policy: &RetryPolicy) -> f64 {
    if let Some(v) = headers.get(RETRY_AFTER) {
        if let Ok(text) = v.to_str() {
            let text = text.trim();
            if let Ok(secs) = text.parse::<f64>() {
                return secs.clamp(0.0, policy.max_secs);
            }
            if let Ok(date) = httpdate::parse_http_date(text) {
                let wait = date
                    .duration_since(std::time::SystemTime::now())
                    .map(|d| d.as_secs_f64())
                    .unwrap_or(0.0);
                return wait.clamp(0.0, policy.max_secs);
            }
        }
    }
    let exp = attempt.saturating_sub(1).min(32);
    (policy.base_secs * 2f64.powi(exp as i32)).min(policy.max_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_f027_no_retry_on_404() {
        assert!(!should_retry(Some(404)));
        assert!(!should_retry(Some(400)));
        assert!(!should_retry(Some(301)));
    }

    #[test]
    fn spec_f027_retries_429_5xx_and_transport() {
        assert!(should_retry(None));
        assert!(should_retry(Some(429)));
        assert!(should_retry(Some(500)));
        assert!(should_retry(Some(503)));
        assert!(should_retry(Some(599)));
    }

    #[test]
    fn spec_f027_retry_after_numeric_clamped() {
        let mut h = HeaderMap::new();
        h.insert(RETRY_AFTER, "5".parse().expect("header"));
        let p = RetryPolicy::default();
        assert_eq!(retry_delay_secs(1, &h, &p), 5.0);
        h.insert(RETRY_AFTER, "9999".parse().expect("header"));
        assert_eq!(retry_delay_secs(1, &h, &p), 30.0);
    }

    #[test]
    fn spec_f027_backoff_doubles_capped_at_max() {
        let h = HeaderMap::new();
        let p = RetryPolicy::default();
        assert_eq!(retry_delay_secs(1, &h, &p), 1.0);
        assert_eq!(retry_delay_secs(2, &h, &p), 2.0);
        assert_eq!(retry_delay_secs(3, &h, &p), 4.0);
        assert_eq!(retry_delay_secs(9, &h, &p), 30.0);
    }

    #[test]
    fn spec_f027_default_policy_matches_spec() {
        let p = RetryPolicy::default();
        assert_eq!(p.max_attempts, 1);
        assert_eq!(p.base_secs, 1.0);
        assert_eq!(p.max_secs, 30.0);
    }
}
