//! Timestamps for file names, without a date crate: UTC civil time from
//! Unix milliseconds (Howard Hinnant's days-from-civil inverse).

use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// `yyyymmdd-hhmmss` in UTC. Seconds keep two backups in one minute apart.
pub(crate) fn stamp(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, m, s) = (rem / 3600, rem % 3600 / 60, rem % 60);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}{month:02}{d:02}-{h:02}{m:02}{s:02}")
}

#[cfg(test)]
mod tests {
    #[test]
    fn stamps_are_utc_civil_time() {
        assert_eq!(super::stamp(0), "19700101-000000");
        // 2026-11-13 21:13:04 UTC
        assert_eq!(super::stamp(1_794_604_384_000), "20261113-211304");
        assert_eq!(super::stamp(951_782_400_000), "20000229-000000");
    }
}
