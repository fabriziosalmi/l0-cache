//! Self-contained UTC date/time helpers (no external chrono dependency):
//! RFC 3339 formatting/parsing (second resolution) and `--since` duration parsing.

/// Parse a duration string like "7d", "24h", "30m" into seconds.
pub(crate) fn parse_since(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut chars = s.chars();
    let unit = chars.next_back()?;
    let num_str = chars.as_str();
    let num: u64 = num_str.parse().ok()?;
    match unit {
        'd' => Some(num.saturating_mul(86400)),
        'h' => Some(num.saturating_mul(3600)),
        'm' => Some(num.saturating_mul(60)),
        's' => Some(num),
        _ => None,
    }
}

/// Get current UTC time in RFC3339 format.
pub(crate) fn rfc3339_now() -> String {
    let now = std::time::SystemTime::now();
    let duration = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    to_rfc3339(duration.as_secs())
}

/// Format a Unix timestamp (seconds since epoch) as UTC RFC3339: `YYYY-MM-DDTHH:MM:SSZ`.
pub(crate) fn to_rfc3339(secs: u64) -> String {
    const SECS_PER_DAY: u64 = 86400;
    const SECS_PER_HOUR: u64 = 3600;
    const SECS_PER_MINUTE: u64 = 60;

    let days = secs / SECS_PER_DAY;
    let secs_of_day = secs % SECS_PER_DAY;

    let mut year = 1970;
    let mut days_left = days;

    loop {
        let is_leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
        let days_in_year = if is_leap { 366 } else { 365 };
        if days_left < days_in_year {
            break;
        }
        days_left -= days_in_year;
        year += 1;
    }

    let is_leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
    let month_days = if is_leap {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut month = 1;
    for &d in &month_days {
        if days_left < d {
            break;
        }
        days_left -= d;
        month += 1;
    }

    let day = days_left + 1;
    let hour = secs_of_day / SECS_PER_HOUR;
    let minute = (secs_of_day % SECS_PER_HOUR) / SECS_PER_MINUTE;
    let second = secs_of_day % SECS_PER_MINUTE;

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hour, minute, second
    )
}

/// Parse a UTC or offset RFC3339 timestamp into a Unix timestamp (seconds since epoch).
pub(crate) fn parse_rfc3339_to_secs(s: &str) -> Option<u64> {
    let s = s.trim();
    if !s.is_ascii() {
        return None;
    }
    if s.len() < 19 {
        return None;
    }
    let year: u64 = s[0..4].parse().ok()?;
    if s.chars().nth(4)? != '-' {
        return None;
    }
    let month: u64 = s[5..7].parse().ok()?;
    if s.chars().nth(7)? != '-' {
        return None;
    }
    let day: u64 = s[8..10].parse().ok()?;
    if s.chars().nth(10)? != 'T' {
        return None;
    }
    let hour: u64 = s[11..13].parse().ok()?;
    if s.chars().nth(13)? != ':' {
        return None;
    }
    let minute: u64 = s[14..16].parse().ok()?;
    if s.chars().nth(16)? != ':' {
        return None;
    }
    let second: u64 = s[17..19].parse().ok()?;

    let mut tz_char = 'Z';
    let mut tz_idx = s.len();
    for (i, c) in s.char_indices().skip(19) {
        if c == 'Z' || c == '+' || c == '-' {
            tz_char = c;
            tz_idx = i;
            break;
        }
    }

    let mut days = 0u64;
    for y in 1970..year {
        let is_leap = (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0);
        days += if is_leap { 366 } else { 365 };
    }

    let is_leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
    let month_days = if is_leap {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    if !(1..=12).contains(&month) {
        return None;
    }
    for m in 1..month {
        days += month_days[m as usize - 1];
    }
    if !(1..=month_days[month as usize - 1]).contains(&day) {
        return None;
    }
    days += day - 1;

    let mut total_secs = days * 86400 + hour * 3600 + minute * 60 + second;

    if tz_char == '+' || tz_char == '-' {
        let offset_str = &s[tz_idx + 1..];
        let parts: Vec<&str> = offset_str.split(':').collect();
        let off_hour: u64;
        let mut off_min: u64 = 0;
        if parts.len() == 2 {
            off_hour = parts[0].parse().ok()?;
            off_min = parts[1].parse().ok()?;
        } else if offset_str.len() == 4 {
            off_hour = offset_str[0..2].parse().ok()?;
            off_min = offset_str[2..4].parse().ok()?;
        } else if offset_str.len() == 2 {
            off_hour = offset_str.parse().ok()?;
        } else {
            return None;
        }
        let offset_secs = off_hour * 3600 + off_min * 60;
        if tz_char == '+' {
            total_secs = total_secs.checked_sub(offset_secs)?;
        } else {
            total_secs = total_secs.checked_add(offset_secs)?;
        }
    }

    Some(total_secs)
}
