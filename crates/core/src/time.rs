//! Minimal time handling on `i64` unix seconds (UTC). Replaces chrono/time:
//! RFC 3339 parse/format, civil date math, and local-time conversion through
//! the Windows time zone API (DST-correct).

use std::time::{SystemTime, UNIX_EPOCH};

/// Current unix time in seconds.
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Days since 1970-01-01 for a proleptic Gregorian date (H. Hinnant).
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Inverse of [`days_from_civil`]: (year, month 1-12, day 1-31).
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Broken-down wall-clock time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Civil {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    /// 0 = Sunday.
    pub weekday: u32,
}

pub fn civil_utc(ts: i64) -> Civil {
    let days = ts.div_euclid(86_400);
    let secs = ts.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    Civil {
        year,
        month,
        day,
        hour: (secs / 3600) as u32,
        minute: ((secs % 3600) / 60) as u32,
        second: (secs % 60) as u32,
        weekday: (days + 4).rem_euclid(7) as u32,
    }
}

/// Wall-clock time in the user's Windows time zone (falls back to UTC).
pub fn civil_local(ts: i64) -> Civil {
    #[cfg(windows)]
    {
        if let Some(c) = win_local(ts) {
            return c;
        }
    }
    civil_utc(ts)
}

#[cfg(windows)]
fn win_local(ts: i64) -> Option<Civil> {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::Time::SystemTimeToTzSpecificLocalTime;
    let u = civil_utc(ts);
    if !(1601..=30827).contains(&u.year) {
        return None;
    }
    let utc = SYSTEMTIME {
        wYear: u.year as u16,
        wMonth: u.month as u16,
        wDayOfWeek: u.weekday as u16,
        wDay: u.day as u16,
        wHour: u.hour as u16,
        wMinute: u.minute as u16,
        wSecond: u.second as u16,
        wMilliseconds: 0,
    };
    let mut local: SYSTEMTIME = unsafe { std::mem::zeroed() };
    // SAFETY: both pointers reference valid SYSTEMTIME values; a null time
    // zone means "the currently active time zone".
    let ok = unsafe { SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local) };
    if ok == 0 {
        return None;
    }
    Some(Civil {
        year: local.wYear as i64,
        month: local.wMonth as u32,
        day: local.wDay as u32,
        hour: local.wHour as u32,
        minute: local.wMinute as u32,
        second: local.wSecond as u32,
        weekday: local.wDayOfWeek as u32,
    })
}

/// `2026-11-12T21:59:59Z` (second precision, always UTC).
pub fn format_rfc3339(ts: i64) -> String {
    let c = civil_utc(ts);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        c.year, c.month, c.day, c.hour, c.minute, c.second
    )
}

/// Parses RFC 3339 / ISO 8601 date-times such as
/// `2026-11-12T21:59:59.853879+00:00`, `...Z`, or a bare `2026-12-01`
/// (treated as midnight UTC). Fractional seconds are truncated.
pub fn parse_rfc3339(s: &str) -> Option<i64> {
    let s = s.trim();
    let b = s.as_bytes();
    let num = |r: std::ops::Range<usize>| -> Option<i64> {
        let part = s.get(r)?;
        if part.is_empty() || !part.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        part.parse().ok()
    };
    if b.len() < 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    let date = days_from_civil(y, mo as u32, d as u32) * 86_400;
    if b.len() == 10 {
        return Some(date);
    }
    if !matches!(b[10], b'T' | b't' | b' ') || b.len() < 19 || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let (h, mi, sec) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    let mut i = 19;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == start {
            return None;
        }
    }
    let offset = match b.get(i) {
        None => 0, // no zone: assume UTC
        Some(b'Z') | Some(b'z') if i + 1 == b.len() => 0,
        Some(&sign @ (b'+' | b'-')) => {
            if b.len() != i + 6 || b[i + 3] != b':' {
                return None;
            }
            let oh = num(i + 1..i + 3)?;
            let om = num(i + 4..i + 6)?;
            let o = oh * 3600 + om * 60;
            if sign == b'+' {
                o
            } else {
                -o
            }
        }
        _ => return None,
    };
    Some(date + h * 3600 + mi * 60 + sec - offset)
}

/// Serde adapter: `Option<i64>` unix seconds <-> RFC 3339 string or null.
/// Deserialization also accepts plain integers (unix seconds).
pub mod opt_rfc3339 {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &Option<i64>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(ts) => s.serialize_str(&super::format_rfc3339(*ts)),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
        let v = Option::<serde_json::Value>::deserialize(d)?;
        Ok(match v {
            Some(serde_json::Value::String(s)) => super::parse_rfc3339(&s),
            Some(serde_json::Value::Number(n)) => n.as_i64(),
            _ => None,
        })
    }
}

/// Serde adapter for a required timestamp.
pub mod rfc3339 {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &i64, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&super::format_rfc3339(*v))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
        let v = serde_json::Value::deserialize(d)?;
        match v {
            serde_json::Value::String(s) => super::parse_rfc3339(&s)
                .ok_or_else(|| serde::de::Error::custom("invalid RFC 3339 timestamp")),
            serde_json::Value::Number(n) => n
                .as_i64()
                .ok_or_else(|| serde::de::Error::custom("invalid unix timestamp")),
            _ => Err(serde::de::Error::custom("expected timestamp")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_round_trip() {
        for days in [-800_000, -1, 0, 1, 10_957, 20_404, 2_932_896] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days, "{y}-{m}-{d}");
        }
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
    }

    #[test]
    fn parses_provider_formats() {
        // Claude: microseconds + numeric offset
        assert_eq!(
            parse_rfc3339("2026-11-12T21:59:59.853879+00:00"),
            parse_rfc3339("2026-11-12T21:59:59Z")
        );
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339("1970-01-01T01:00:00+01:00"), Some(0));
        assert_eq!(parse_rfc3339("1969-12-31T19:00:00-05:00"), Some(0));
        assert_eq!(parse_rfc3339("2026-12-01"), Some(days_from_civil(2026, 12, 1) * 86_400));
        assert_eq!(parse_rfc3339("2024-02-29T12:00:00"), Some(1_709_208_000));
    }

    #[test]
    fn rejects_garbage() {
        for s in ["", "nope", "2026-13-01", "2026-11-12T25:00:00Z", "2026-11-12T10:00:00+0100", "2026-11-12T10:00:00.Z"] {
            assert_eq!(parse_rfc3339(s), None, "{s}");
        }
    }

    #[test]
    fn format_round_trip() {
        let ts = 1_794_531_839;
        assert_eq!(parse_rfc3339(&format_rfc3339(ts)), Some(ts));
        assert_eq!(format_rfc3339(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn weekday() {
        assert_eq!(civil_utc(0).weekday, 4); // Thursday
        assert_eq!(civil_utc(parse_rfc3339("2026-11-13T00:00:00Z").unwrap()).weekday, 5);
    }
}
