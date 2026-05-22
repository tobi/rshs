use std::time::{SystemTime, UNIX_EPOCH};

/// RFC 850 / RFC 1036 date format used in HTTP `Last-Modified` headers
/// by older clients.
///
/// Format: `DD-Mon-YYYY HH:MM` (2-digit day, abbreviated month, 4-digit year,
/// 24-hour time). Returns an empty string for pre-Unix-epoch timestamps.
///
/// ```text
/// UNIX_EPOCH                       → "01-Jan-1970 00:00"
/// UNIX_EPOCH + 1716134400 secs     → "19-May-2024 16:00"
/// ```
pub fn format_rfc850(st: SystemTime) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];

    let duration = match st.duration_since(UNIX_EPOCH) {
        Ok(d) => d,
        Err(_) => return String::new(),
    };

    let total_secs = duration.as_secs() as i64;
    let days = (total_secs / 86400) as i32;
    let time_of_day = total_secs % 86400;
    let hours = time_of_day / 3600;
    let mins = (time_of_day % 3600) / 60;

    let (year, month, day) = civil_from_days(days);

    format!(
        "{day:02}-{month_abbr}-{year:04} {hours:02}:{mins:02}",
        month_abbr = MONTHS[(month - 1) as usize]
    )
}

/// RFC 1123 / RFC 7231 date format, the preferred HTTP date format.
///
/// Format: `WD, DD Mon YYYY HH:MM:SS GMT` (day-of-week name, 2-digit day,
/// abbreviated month, 4-digit year, 24-hour time). Returns an empty string
/// for pre-Unix-epoch timestamps.
///
/// ```text
/// UNIX_EPOCH                       → "Thu, 01 Jan 1970 00:00:00 GMT"
/// UNIX_EPOCH + 1716134400 secs     → "Sun, 19 May 2024 16:00:00 GMT"
/// ```
pub fn format_rfc1123(st: SystemTime) -> String {
    const DAY_NAMES: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];

    let duration = match st.duration_since(UNIX_EPOCH) {
        Ok(d) => d,
        Err(_) => return String::new(),
    };

    let total_secs = duration.as_secs() as i64;
    let days = (total_secs / 86400) as i32;
    let time_of_day = total_secs % 86400;
    let hours = time_of_day / 3600;
    let mins = (time_of_day % 3600) / 60;
    let secs = time_of_day % 60;

    let (year, month, day) = civil_from_days(days);
    let dow = ((days as i64 + 4) % 7) as usize; // Unix epoch was a Thursday

    format!(
        "{}, {:02} {} {:04} {:02}:{:02}:{:02} GMT",
        DAY_NAMES[dow],
        day,
        MONTHS[(month - 1) as usize],
        year,
        hours,
        mins,
        secs
    )
}

/// RFC 3339 / ISO 8601 date format used in WebDAV `creationdate` properties.
///
/// Format: `YYYY-MM-DDTHH:MM:SSZ` (ISO 8601 calendar date, UTC designator).
/// Returns an empty string for pre-Unix-epoch timestamps.
///
/// ```text
/// UNIX_EPOCH                       → "1970-01-01T00:00:00Z"
/// UNIX_EPOCH + 1716134400 secs     → "2024-05-19T16:00:00Z"
/// ```
pub fn format_rfc3339(st: SystemTime) -> String {
    let duration = match st.duration_since(UNIX_EPOCH) {
        Ok(d) => d,
        Err(_) => return String::new(),
    };

    let total_secs = duration.as_secs() as i64;
    let days = (total_secs / 86400) as i32;
    let time_of_day = total_secs % 86400;
    let hours = time_of_day / 3600;
    let mins = (time_of_day % 3600) / 60;
    let secs = time_of_day % 60;

    let (year, month, day) = civil_from_days(days);

    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{mins:02}:{secs:02}Z")
}

/// Days-since-Unix-epoch to (year, month, day).
///
/// Howard Hinnant's `civil_from_days` algorithm — a branchless calendar
/// conversion that forms the basis of C++20 `std::chrono` date support.
///
/// Source: <http://howardhinnant.github.io/date_algorithms.html>
fn civil_from_days(z: i32) -> (i32, u32, u32) {
    let z = z as i64 + 719468;
    let era = if z >= 0 {
        z / 146097
    } else {
        (z - 146096) / 146097
    };
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = (yoe + era as u64 * 400) as i64;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn unix_epoch() -> SystemTime {
        UNIX_EPOCH
    }

    fn timestamp(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn test_civil_from_days_epoch() {
        let (y, m, d) = civil_from_days(0);
        assert_eq!((y, m, d), (1970, 1, 1));
    }

    #[test]
    fn test_civil_from_days_known_date() {
        let (y, m, d) = civil_from_days(10957);
        assert_eq!((y, m, d), (2000, 1, 1));
    }

    #[test]
    fn test_civil_from_days_epoch_plus_one_year() {
        let (y, m, d) = civil_from_days(365);
        assert_eq!((y, m, d), (1971, 1, 1));
    }

    #[test]
    fn test_civil_from_days_feb_29_in_leap_year() {
        let (y, m, d) = civil_from_days(789);
        assert_eq!((y, m, d), (1972, 2, 29));
    }

    #[test]
    fn test_format_rfc3339_epoch() {
        let s = format_rfc3339(unix_epoch());
        assert_eq!(s, "1970-01-01T00:00:00Z");
    }

    #[test]
    fn test_format_rfc3339_known_date() {
        let s = format_rfc3339(timestamp(1716134400));
        assert_eq!(s, "2024-05-19T16:00:00Z");
    }

    #[test]
    fn test_format_rfc1123_epoch() {
        let s = format_rfc1123(unix_epoch());
        assert_eq!(s, "Thu, 01 Jan 1970 00:00:00 GMT");
    }

    #[test]
    fn test_format_rfc1123_known_date() {
        let s = format_rfc1123(timestamp(1716134400));
        assert_eq!(s, "Sun, 19 May 2024 16:00:00 GMT");
    }

    #[test]
    fn test_format_rfc850_epoch() {
        let s = format_rfc850(unix_epoch());
        assert_eq!(s, "01-Jan-1970 00:00");
    }

    #[test]
    fn test_format_rfc850_known_date() {
        let s = format_rfc850(timestamp(1716134400));
        assert_eq!(s, "19-May-2024 16:00");
    }

    #[test]
    fn test_format_before_epoch_returns_empty() {
        let before = UNIX_EPOCH - Duration::from_secs(86400);
        assert_eq!(format_rfc3339(before), "");
        assert_eq!(format_rfc1123(before), "");
        assert_eq!(format_rfc850(before), "");
    }
}
