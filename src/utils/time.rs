use std::time::{SystemTime, UNIX_EPOCH};

pub fn format_modified(st: SystemTime) -> String {
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
