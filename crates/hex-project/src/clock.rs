//! Timestamps and identifiers used by the project layer.
//!
//! The dynamics and analysis crates deliberately stay free of a date library, so
//! this module provides the ISO 8601 formatting and identifier generation the
//! on-disk artifacts need.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds since the Unix epoch, or zero when the clock is unavailable.
pub fn unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// A civil date and time derived from a Unix timestamp.
///
/// Implemented directly rather than through a dependency, using the standard
/// days-to-civil algorithm. `chrono` is available, but a project file timestamp
/// should not depend on a timezone database being present.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct UtcDateTime {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub millisecond: u32,
}

impl UtcDateTime {
    /// Break a Unix timestamp with milliseconds into civil fields.
    pub fn from_unix_millis(millis: i64) -> Self {
        let total_seconds = millis.div_euclid(1000);
        let millisecond = millis.rem_euclid(1000) as u32;
        let days = total_seconds.div_euclid(86_400);
        let seconds_of_day = total_seconds.rem_euclid(86_400);

        let (year, month, day) = civil_from_days(days);
        Self {
            year,
            month,
            day,
            hour: (seconds_of_day / 3600) as u32,
            minute: ((seconds_of_day % 3600) / 60) as u32,
            second: (seconds_of_day % 60) as u32,
            millisecond,
        }
    }

    /// The current UTC time.
    pub fn now() -> Self {
        Self::from_unix_millis(now_unix_millis())
    }

    /// An ISO 8601 rendering with millisecond precision and a `Z` suffix.
    pub fn to_iso8601(&self) -> String {
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
            self.year, self.month, self.day, self.hour, self.minute, self.second, self.millisecond
        )
    }

    /// A compact rendering suitable for a file name.
    pub fn to_file_stamp(&self) -> String {
        format!(
            "{:04}{:02}{:02}-{:02}{:02}{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }

    /// A human-readable rendering for the UI.
    pub fn to_display(&self) -> String {
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }
}

/// Milliseconds since the Unix epoch.
pub fn now_unix_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// The current time as an ISO 8601 string.
pub fn now_iso8601() -> String {
    UtcDateTime::now().to_iso8601()
}

/// The current time as a file-name stamp.
pub fn now_file_stamp() -> String {
    UtcDateTime::now().to_file_stamp()
}

/// Convert a day count since the Unix epoch into a civil date.
///
/// This is Howard Hinnant's `civil_from_days`, which is exact for the whole range
/// of `i64` days that matter here and has no leap-second or timezone input.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (year, m as u32, d as u32)
}

/// A monotonic counter used to make identifiers unique within a process.
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A short unique identifier.
///
/// Built from the process time and a counter rather than a random UUID, so two
/// identifiers created in the same millisecond still differ and the value is
/// stable enough to appear in a file name.
pub fn new_id() -> String {
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let millis = now_unix_millis() as u64;
    format!(
        "{:012x}{:06x}",
        millis & 0xffff_ffff_ffff,
        counter & 0xff_ffff
    )
}

/// A UUID-shaped identifier, for fields the specification describes as a UUID.
pub fn new_uuid_like() -> String {
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let millis = now_unix_millis() as u64;
    let mixed = millis
        .wrapping_mul(0x9e37_79b9_7f4a_7c15)
        .wrapping_add(counter.wrapping_mul(0xbf58_476d_1ce4_e5b9));
    format!(
        "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        (mixed >> 32) as u32,
        (mixed >> 16) as u16,
        (mixed & 0xfff) as u16,
        ((mixed >> 48) as u16 & 0x3fff) | 0x8000,
        mixed & 0xffff_ffff_ffff
    )
}

/// A stable hash of a byte slice, used for content fingerprints in metadata.
pub fn content_hash(bytes: &[u8]) -> String {
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(bytes);
    format!("crc32:{:08x}", hasher.finalize())
}

/// A stable hash of a file's contents.
pub fn file_hash(path: &std::path::Path) -> Result<String, crate::error::ProjectError> {
    let bytes = std::fs::read(path).map_err(|e| crate::error::ProjectError::io(path, e))?;
    Ok(content_hash(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_converts_to_the_documented_civil_date() {
        let t = UtcDateTime::from_unix_millis(0);
        assert_eq!((t.year, t.month, t.day), (1970, 1, 1));
        assert_eq!((t.hour, t.minute, t.second), (0, 0, 0));
    }

    #[test]
    fn known_timestamps_convert_correctly() {
        // 2000-03-01T00:00:00Z is 951868800 seconds.
        let t = UtcDateTime::from_unix_millis(951_868_800_000);
        assert_eq!((t.year, t.month, t.day), (2000, 3, 1));

        // 2024-02-29T12:34:56.789Z, a leap day.
        let millis = 1_709_210_096_789i64;
        let t = UtcDateTime::from_unix_millis(millis);
        assert_eq!((t.year, t.month, t.day), (2024, 2, 29));
        assert_eq!(
            (t.hour, t.minute, t.second, t.millisecond),
            (12, 34, 56, 789)
        );
    }

    #[test]
    fn negative_timestamps_are_handled() {
        // 1969-12-31T23:59:59Z.
        let t = UtcDateTime::from_unix_millis(-1000);
        assert_eq!((t.year, t.month, t.day), (1969, 12, 31));
        assert_eq!((t.hour, t.minute, t.second), (23, 59, 59));
    }

    #[test]
    fn iso_rendering_has_the_expected_shape() {
        let t = UtcDateTime {
            year: 2024,
            month: 2,
            day: 29,
            hour: 12,
            minute: 34,
            second: 56,
            millisecond: 7,
        };
        assert_eq!(t.to_iso8601(), "2024-02-29T12:34:56.007Z");
        assert_eq!(t.to_file_stamp(), "20240229-123456");
        assert!(t.to_display().contains("UTC"));
    }

    #[test]
    fn now_is_a_plausible_recent_value() {
        let now = UtcDateTime::now();
        assert!(now.year >= 2024, "year {}", now.year);
        assert!((1..=12).contains(&now.month));
        assert!((1..=31).contains(&now.day));
        let iso = now_iso8601();
        assert_eq!(iso.len(), 24, "{}", iso);
        assert!(iso.ends_with('Z'));
        assert_eq!(now_file_stamp().len(), 15);
    }

    #[test]
    fn identifiers_are_unique_and_ordered_by_creation() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1000 {
            assert!(seen.insert(new_id()));
        }
    }

    #[test]
    fn uuids_have_the_expected_shape_and_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..500 {
            let id = new_uuid_like();
            assert_eq!(id.len(), 36, "{}", id);
            assert_eq!(id.chars().filter(|c| *c == '-').count(), 4);
            assert_eq!(id.chars().nth(14), Some('4'), "{}", id);
            assert!(seen.insert(id));
        }
    }

    #[test]
    fn content_hash_is_stable_and_distinguishes_content() {
        let a = content_hash(b"hello");
        let b = content_hash(b"hello");
        let c = content_hash(b"hello!");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!(a.starts_with("crc32:"));
    }

    #[test]
    fn file_hash_matches_the_content_hash() {
        let dir = std::env::temp_dir().join(format!("hexadof-hash-{}", new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("value.bin");
        std::fs::write(&path, b"payload").unwrap();
        assert_eq!(file_hash(&path).unwrap(), content_hash(b"payload"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_hash_reports_a_missing_file() {
        let missing = std::env::temp_dir().join(format!("hexadof-nohash-{}", new_id()));
        assert!(file_hash(&missing).is_err());
    }

    #[test]
    fn unix_seconds_is_recent() {
        let s = unix_seconds();
        assert!(s > 1_700_000_000, "seconds {}", s);
    }
}
