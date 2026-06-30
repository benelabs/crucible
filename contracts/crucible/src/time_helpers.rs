//! Calendar-aware utilities for advancing ledger timestamps in tests.
//!
//! Unlike fixed-second [`Duration`](crate::env::Duration) helpers, these functions
//! account for variable month lengths and leap years.

/// Returns `true` if `year` is a leap year in the Gregorian calendar.
pub fn is_leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

/// Returns the number of days in `month` (1–12) of `year`.
pub fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => panic!("invalid month: {month}"),
    }
}

/// Decomposes a UNIX timestamp into UTC `(year, month, day, hour, minute, second)`.
///
/// Uses the civil-from-days algorithm from
/// [Howard Hinnant](https://howardhinnant.github.io/date_algorithms.html).
pub fn unix_to_datetime(ts: u64) -> (i32, u32, u32, u32, u32, u32) {
    let mut remaining = ts;
    let second = (remaining % 60) as u32;
    remaining /= 60;
    let minute = (remaining % 60) as u32;
    remaining /= 60;
    let hour = (remaining % 24) as u32;
    let days = remaining / 24;

    let z = days as i64 + 719_468;
    let era = if z >= 0 {
        z / 146_097
    } else {
        (z - 146_096) / 146_097
    };
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i32 + era as i32 * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if mp < 10 { y } else { y + 1 };

    (year, m, d, hour, minute, second)
}

/// Composes a UNIX timestamp from UTC `(year, month, day, hour, minute, second)`.
pub fn datetime_to_unix(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> u64 {
    let y = if month <= 2 { year - 1 } else { year };
    let m = if month <= 2 { month + 9 } else { month - 3 };
    let era = if y >= 0 { y / 400 } else { (y - 399) / 400 };
    let yoe = (y - era * 400) as u32;
    let doy = (153 * m + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era as i64 * 146_097 + doe as i64 - 719_468;

    days as u64 * 86_400 + hour as u64 * 3_600 + minute as u64 * 60 + second as u64
}

/// Advances a UNIX timestamp by `months` using calendar month arithmetic.
///
/// When the source day does not exist in the target month (e.g. Jan 31 → Feb),
/// the result is clamped to the last valid day of that month.
pub fn add_months(ts: u64, months: u32) -> u64 {
    let (year, month, day, hour, minute, second) = unix_to_datetime(ts);
    let total_months = year as i64 * 12 + month as i64 - 1 + months as i64;
    let new_year = (total_months / 12) as i32;
    let new_month = (total_months % 12 + 1) as u32;
    let max_day = days_in_month(new_year, new_month);
    let new_day = day.min(max_day);

    datetime_to_unix(new_year, new_month, new_day, hour, minute, second)
}

/// Advances a UNIX timestamp by `years` using calendar year arithmetic.
///
/// When the source day does not exist in the target year (e.g. Feb 29 → non-leap year),
/// the result is clamped to Feb 28.
pub fn add_years(ts: u64, years: u32) -> u64 {
    let (year, month, day, hour, minute, second) = unix_to_datetime(ts);
    let new_year = year + years as i32;
    let max_day = days_in_month(new_year, month);
    let new_day = day.min(max_day);

    datetime_to_unix(new_year, month, new_day, hour, minute, second)
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2024-01-31 12:30:45 UTC
    const JAN_31_2024: u64 = 1_706_704_245;
    // 2024-02-29 12:30:45 UTC (leap day)
    const FEB_29_2024: u64 = 1_709_209_845;
    // 2023-01-31 00:00:00 UTC
    const JAN_31_2023: u64 = 1_675_123_200;
    // 2024-03-15 08:00:00 UTC
    const MAR_15_2024: u64 = 1_710_489_600;

    #[test]
    fn unix_round_trip_preserves_datetime() {
        let cases = [
            (0, (1970, 1, 1, 0, 0, 0)),
            (JAN_31_2024, (2024, 1, 31, 12, 30, 45)),
            (FEB_29_2024, (2024, 2, 29, 12, 30, 45)),
            (MAR_15_2024, (2024, 3, 15, 8, 0, 0)),
        ];

        for (ts, (y, m, d, h, min, s)) in cases {
            assert_eq!(unix_to_datetime(ts), (y, m, d, h, min, s));
            assert_eq!(datetime_to_unix(y, m, d, h, min, s), ts);
        }
    }

    #[test]
    fn add_months_clamps_end_of_month() {
        // Jan 31 + 1 month → Feb 29 (leap year)
        assert_eq!(add_months(JAN_31_2024, 1), datetime_to_unix(2024, 2, 29, 12, 30, 45));
        // Jan 31 + 1 month → Feb 28 (non-leap year)
        assert_eq!(add_months(JAN_31_2023, 1), datetime_to_unix(2023, 2, 28, 0, 0, 0));
        // Mar 15 + 1 month → Apr 15
        assert_eq!(add_months(MAR_15_2024, 1), datetime_to_unix(2024, 4, 15, 8, 0, 0));
    }

    #[test]
    fn add_months_handles_multiple_months() {
        assert_eq!(add_months(MAR_15_2024, 12), datetime_to_unix(2025, 3, 15, 8, 0, 0));
    }

    #[test]
    fn add_years_preserves_date() {
        assert_eq!(add_years(MAR_15_2024, 1), datetime_to_unix(2025, 3, 15, 8, 0, 0));
    }

    #[test]
    fn add_years_clamps_leap_day() {
        // Feb 29, 2024 + 1 year → Feb 28, 2025
        assert_eq!(add_years(FEB_29_2024, 1), datetime_to_unix(2025, 2, 28, 12, 30, 45));
        // Feb 29, 2024 + 4 years → Feb 29, 2028
        assert_eq!(add_years(FEB_29_2024, 4), datetime_to_unix(2028, 2, 29, 12, 30, 45));
    }

    #[test]
    fn is_leap_year_examples() {
        assert!(is_leap_year(2000));
        assert!(is_leap_year(2024));
        assert!(!is_leap_year(1900));
        assert!(!is_leap_year(2023));
    }
}
