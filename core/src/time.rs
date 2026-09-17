//! Timestamp formatting without a date library.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Formats a time as RFC 3339 UTC with second precision, e.g. `2026-09-16T14:02:11Z`.
pub fn iso_utc(t: SystemTime) -> String {
    let secs = match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs() as i64) - i64::from(e.duration().subsec_nanos() > 0),
    };
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

pub fn now_iso() -> String {
    iso_utc(SystemTime::now())
}

/// Nanoseconds since the Unix epoch, for change detection.
pub fn unix_nanos(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_nanos() as i64,
        Err(e) => -(e.duration().as_nanos() as i64),
    }
}

pub fn from_unix_nanos(n: i64) -> SystemTime {
    if n >= 0 {
        UNIX_EPOCH + Duration::from_nanos(n as u64)
    } else {
        UNIX_EPOCH - Duration::from_nanos(n.unsigned_abs())
    }
}

/// Converts an EXIF date (`2001:02:03 04:05:06`) and optional offset (`+01:00`)
/// to RFC 3339. Without an offset the result is a local time with no zone.
pub fn exif_to_iso(date_time: &str, offset: Option<&str>) -> Option<String> {
    let s = date_time.trim().trim_end_matches('\0');
    let b = s.as_bytes();
    if b.len() < 19
        || b[4] != b':'
        || b[7] != b':'
        || b[10] != b' '
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    let digits = |r: std::ops::Range<usize>| s[r].bytes().all(|c| c.is_ascii_digit());
    if ![0..4, 5..7, 8..10, 11..13, 14..16, 17..19]
        .into_iter()
        .all(digits)
        || &s[0..4] == "0000"
    {
        return None;
    }
    let mut out = format!("{}-{}-{}T{}", &s[0..4], &s[5..7], &s[8..10], &s[11..19]);
    if let Some(o) = offset.map(str::trim) {
        let ob = o.as_bytes();
        if ob.len() == 6 && (ob[0] == b'+' || ob[0] == b'-') && ob[3] == b':' {
            out.push_str(o);
        }
    }
    Some(out)
}

/// Days since 1970-01-01 to (year, month, day). Howard Hinnant's algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_utc() {
        assert_eq!(iso_utc(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        assert_eq!(
            iso_utc(UNIX_EPOCH + Duration::from_secs(951_782_400 + 3661)),
            "2000-02-29T01:01:01Z"
        );
        assert_eq!(
            iso_utc(UNIX_EPOCH - Duration::from_secs(1)),
            "1969-12-31T23:59:59Z"
        );
    }

    #[test]
    fn converts_exif_dates() {
        assert_eq!(
            exif_to_iso("2001:02:03 04:05:06", Some("+01:00")).as_deref(),
            Some("2001-02-03T04:05:06+01:00")
        );
        assert_eq!(
            exif_to_iso("2001:02:03 04:05:06\0", None).as_deref(),
            Some("2001-02-03T04:05:06")
        );
        assert_eq!(exif_to_iso("0000:00:00 00:00:00", None), None);
        assert_eq!(exif_to_iso("    :  :     :  :  ", None), None);
    }

    #[test]
    fn nanos_round_trip() {
        let t = UNIX_EPOCH + Duration::from_nanos(1_234_567_890_123);
        assert_eq!(from_unix_nanos(unix_nanos(t)), t);
    }
}
