//! Rate-limit detection for agent error messages and the cooldown info
//! that we surface back to the UI. Used by the conductor's `run_loop` to
//! distinguish "the model said something we couldn't parse" (which needs
//! human attention) from "we hit the provider's 5-hour limit" (which just
//! needs to wait it out).
//!
//! Detection is conservative — false negatives are preferred over false
//! positives. A missed rate-limit just lands the conductor in regular
//! Paused (user fixes manually), while a false positive would silently
//! waste 30 minutes sleeping when the real fix was a one-line change.

use chrono::Utc;
use serde::{Deserialize, Serialize};

/// What the UI shows while the conductor is sleeping out a rate-limit. The
/// frontend renders a countdown to `retry_at_ms`, then the auto-resumed
/// iteration takes over naturally — no extra signal needed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CooldownInfo {
    /// Unix-ms when the conductor plans to wake up and retry. The UI uses
    /// this to draw a "продолжим через 14:35" countdown.
    pub retry_at_ms: i64,
    /// Truncated original error text from the agent. Shown verbatim so the
    /// user can verify "yep, that's a limit, not something else".
    pub reason: String,
    /// Which iteration is parked in cooldown. Null when not attached to a
    /// specific iteration (defensive — current callers always set this).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iteration_id: Option<String>,
}

/// Fallback wait when the error doesn't tell us a concrete reset time.
/// 30 min strikes the balance between "noticeable lag for the user" and
/// "useful if Claude actually has 4 more hours to recover".
const DEFAULT_COOLDOWN_MS: i64 = 30 * 60 * 1000;
/// Minimum sleep — never schedule a retry for "right now", that produces
/// a tight loop on garbled error messages.
const MIN_COOLDOWN_MS: i64 = 60 * 1000;
/// Hard cap — Claude's 5-hour window plus a buffer. Anything bigger is
/// almost certainly a parse error masquerading as a huge timestamp.
const MAX_COOLDOWN_MS: i64 = 6 * 60 * 60 * 1000;

/// Look for known rate-limit patterns in an agent error message and, if
/// found, return when to retry. The detection list is keyword-based on
/// purpose: provider error JSON varies by SDK version, but the human
/// phrasing of "rate limit" / "usage limit" / "quota" stays stable.
pub fn detect_rate_limit(msg: &str) -> Option<CooldownInfo> {
    let lower = msg.to_ascii_lowercase();
    let signals = [
        "rate limit",
        "rate_limit",
        "ratelimit",
        "usage limit",
        "usage_limit",
        "5-hour limit",
        "5 hour limit",
        "5h limit",
        "quota exceeded",
        "quota_exceeded",
        "too many requests",
        "rate_limit_error",
    ];
    if !signals.iter().any(|s| lower.contains(s)) {
        return None;
    }

    let now = Utc::now().timestamp_millis();
    let retry_at_ms = extract_explicit_retry_ms(msg, now)
        .map(|t| t.clamp(now + MIN_COOLDOWN_MS, now + MAX_COOLDOWN_MS))
        .unwrap_or(now + DEFAULT_COOLDOWN_MS);
    let reason: String = msg.chars().take(400).collect();
    Some(CooldownInfo {
        retry_at_ms,
        reason,
        iteration_id: None,
    })
}

/// Try to pull a concrete reset time out of the error string. Handles the
/// formats Claude / OpenAI have used historically; falls back to `None`
/// when nothing matches (caller then uses the default cooldown).
fn extract_explicit_retry_ms(msg: &str, now_ms: i64) -> Option<i64> {
    let lower = msg.to_ascii_lowercase();

    // Claude classic: "Claude AI usage limit reached|<unix_seconds>"
    if let Some(idx) = lower.find("limit reached|") {
        let tail = &msg[idx + "limit reached|".len()..];
        let digits: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(ts_secs) = digits.parse::<i64>() {
            return Some(ts_secs * 1000);
        }
    }

    // OpenAI-style "try again in N seconds" and friends. Numbers below
    // ~24h are interpreted as seconds; anything bigger is assumed to
    // already be in ms (defensive against future API drift).
    let phrases = ["try again in", "retry after", "retry in", "reset in"];
    for phrase in phrases {
        if let Some(idx) = lower.find(phrase) {
            let tail = &msg[idx + phrase.len()..];
            let digits: String = tail
                .chars()
                .skip_while(|c| !c.is_ascii_digit())
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if let Ok(n) = digits.parse::<i64>() {
                let ms = if n >= 86400 { n } else { n * 1000 };
                return Some(now_ms + ms);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_common_rate_limit_phrases() {
        assert!(detect_rate_limit("Claude AI usage limit reached").is_some());
        assert!(detect_rate_limit("rate_limit_error: too many requests").is_some());
        assert!(detect_rate_limit("HTTP 429: rate limit exceeded").is_some());
        assert!(detect_rate_limit("Quota exceeded for this hour").is_some());
        assert!(detect_rate_limit("5 hour limit reached").is_some());
        assert!(detect_rate_limit("RateLimit hit").is_some());
    }

    #[test]
    fn ignores_unrelated_errors() {
        assert!(detect_rate_limit("PO did not return any story").is_none());
        assert!(detect_rate_limit("git push failed").is_none());
        assert!(detect_rate_limit("model not supported").is_none());
        assert!(detect_rate_limit("invalid_request_error").is_none());
    }

    #[test]
    fn extracts_claude_historical_timestamp() {
        let msg = "Claude AI usage limit reached|1762000000 — try again later";
        let extracted = extract_explicit_retry_ms(msg, 1_700_000_000_000).unwrap();
        assert_eq!(extracted, 1_762_000_000_000);
    }

    #[test]
    fn extracts_try_again_in_seconds() {
        let msg = "Rate limit reached. Please try again in 1234 seconds.";
        let now = 1_700_000_000_000i64;
        let extracted = extract_explicit_retry_ms(msg, now).unwrap();
        assert_eq!(extracted, now + 1234 * 1000);
    }

    #[test]
    fn extracts_retry_after_seconds() {
        let msg = "{\"error\":{\"message\":\"retry after 600 seconds\"}}";
        let now = 1_700_000_000_000i64;
        let extracted = extract_explicit_retry_ms(msg, now).unwrap();
        assert_eq!(extracted, now + 600 * 1000);
    }

    #[test]
    fn falls_back_to_default_cooldown() {
        let info = detect_rate_limit("rate limit hit").unwrap();
        let now = Utc::now().timestamp_millis();
        let dt_min = (info.retry_at_ms - now) / 60_000;
        assert!(
            (25..=35).contains(&dt_min),
            "expected ~30 min, got {dt_min} min"
        );
    }

    #[test]
    fn clamps_huge_retry_at_to_max_cooldown() {
        // Provider claims reset is 30 days away → we cap at 6 hours.
        let msg = format!(
            "rate limit. Claude AI usage limit reached|{}",
            (Utc::now().timestamp() + 30 * 24 * 3600)
        );
        let info = detect_rate_limit(&msg).unwrap();
        let dt_hours = (info.retry_at_ms - Utc::now().timestamp_millis()) / 3_600_000;
        assert!((5..=6).contains(&dt_hours), "expected ~6h cap, got {dt_hours}h");
    }
}
