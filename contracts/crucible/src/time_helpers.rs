//! Calendar-aware utilities for advancing ledger timestamps in tests.
//!
//! # Calendar-Aware vs Pure Timestamp Arithmetic
//!
//! Crucible distinguishes between two timestamp advancement strategies:
//! 1. **Fixed-second Timestamp Arithmetic** (e.g. [`Duration`](crate::env::Duration) and
//!    [`advance_time`](crate::env::MockEnv::advance_time)): Adds exact, fixed second intervals
//!    (1 day = 86,400s, 1 hour = 3,600s). This is pure epoch arithmetic.
//! 2. **Calendar-Aware Arithmetic** (e.g. [`add_months`], [`add_years`],
//!    [`advance_time_by_months`](crate::env::MockEnv::advance_time_by_months), and
//!    [`advance_time_by_years`](crate::env::MockEnv::advance_time_by_years)): Accounts for variable
//!    month lengths (28, 29, 30, 31 days) and Gregorian leap year rules. Target dates that do not exist
//!    in a destination month or year (such as Jan 31 → Feb or Feb 29 → non-leap year) are safely clamped
//!    to the last valid day of that month.
//!
//! All calculations are timezone-independent, operating strictly in UTC seconds.

/// Returns `true` if `year` is a leap year in the Gregorian calendar.
pub fn is_leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

/// Returns the number of days in `month` (1–12) of `year`.
///
/// # Panics
/// Panics if `month` is not in the range 1..=12.
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
///
/// # Panics
/// Panics if `ts` causes integer overflow during conversion.
pub fn unix_to_datetime(ts: u64) -> (i32, u32, u32, u32, u32, u32) {
    let mut remaining = ts;
    let second = (remaining % 60) as u32;
    remaining /= 60;
    let minute = (remaining % 60) as u32;
    remaining /= 60;
    let hour = (remaining % 24) as u32;
    let days = remaining / 24;

    let z = (i64::try_from(days).expect("timestamp overflow in days conversion"))
        .checked_add(719_468)
        .expect("day count calculation overflow");
    let era = if z >= 0 {
        z / 146_097
    } else {
        (z - 146_096) / 146_097
    };
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = (i32::try_from(yoe).expect("year conversion overflow"))
        .checked_add(
            (i32::try_from(era).expect("era conversion overflow"))
                .checked_mul(400)
                .expect("era year overflow"),
        )
        .expect("year arithmetic overflow");
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if mp < 10 {
        y
    } else {
        y.checked_add(1).expect("year increment overflow")
    };

    (year, m, d, hour, minute, second)
}

/// Composes a UNIX timestamp from UTC `(year, month, day, hour, minute, second)`.
///
/// # Panics
/// Panics if `month`, `day`, `hour`, `minute`, or `second` are out of valid range,
/// or if timestamp calculation overflows `u64`.
pub fn datetime_to_unix(
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
) -> u64 {
    assert!((1..=12).contains(&month), "invalid month: {month}");
    let max_day = days_in_month(year, month);
    assert!(
        (1..=max_day).contains(&day),
        "invalid day {day} for month {month} in year {year}"
    );
    assert!(hour < 24, "invalid hour: {hour}");
    assert!(minute < 60, "invalid minute: {minute}");
    assert!(second < 60, "invalid second: {second}");

    let y = if month <= 2 { year - 1 } else { year };
    let m = if month <= 2 { month + 9 } else { month - 3 };
    let era = if y >= 0 { y / 400 } else { (y - 399) / 400 };
    let yoe = (y - era * 400) as u32;
    let doy = (153 * m + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = (era as i64)
        .checked_mul(146_097)
        .expect("days era overflow")
        .checked_add(doe as i64)
        .expect("days doe overflow")
        .checked_sub(719_468)
        .expect("days offset underflow");

    assert!(
        days >= 0,
        "timestamp before UNIX epoch (1970-01-01) not supported"
    );

    (days as u64)
        .checked_mul(86_400)
        .expect("timestamp day multiplication overflow")
        .checked_add((hour as u64) * 3_600)
        .expect("timestamp hour addition overflow")
        .checked_add((minute as u64) * 60)
        .expect("timestamp minute addition overflow")
        .checked_add(second as u64)
        .expect("timestamp second addition overflow")
}

/// Advances a UNIX timestamp by `months` using calendar month arithmetic.
///
/// When the source day does not exist in the target month (e.g. Jan 31 → Feb),
/// the result is clamped to the last valid day of that month.
///
/// # Panics
/// Panics if the resulting year or timestamp overflows.
pub fn add_months(ts: u64, months: u32) -> u64 {
    let (year, month, day, hour, minute, second) = unix_to_datetime(ts);
    let months_i64 = i64::from(months);
    let total_months = (year as i64)
        .checked_mul(12)
        .expect("year overflow in add_months")
        .checked_add(month as i64 - 1)
        .expect("year overflow in add_months")
        .checked_add(months_i64)
        .expect("year overflow in add_months");

    let new_year = i32::try_from(total_months.div_euclid(12)).expect("year overflow in add_months");
    assert!(new_year <= 584_554, "year overflow in add_months");
    let new_month = u32::try_from(total_months.rem_euclid(12) + 1).expect("month conversion error");
    let max_day = days_in_month(new_year, new_month);
    let new_day = day.min(max_day);

    datetime_to_unix(new_year, new_month, new_day, hour, minute, second)
}

/// Advances a UNIX timestamp by `years` using calendar year arithmetic.
///
/// When the source day does not exist in the target year (e.g. Feb 29 → non-leap year),
/// the result is clamped to Feb 28.
///
/// # Panics
/// Panics if the resulting year or timestamp overflows.
pub fn add_years(ts: u64, years: u32) -> u64 {
    let (year, month, day, hour, minute, second) = unix_to_datetime(ts);
    let years_i32 = i32::try_from(years).expect("year overflow in add_years");
    let new_year = year
        .checked_add(years_i32)
        .expect("year overflow in add_years");
    assert!(new_year <= 584_554, "year overflow in add_years");
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
    fn add_months_zero_duration() {
        assert_eq!(add_months(JAN_31_2024, 0), JAN_31_2024);
        assert_eq!(add_months(FEB_29_2024, 0), FEB_29_2024);
    }

    #[test]
    fn add_years_zero_duration() {
        assert_eq!(add_years(JAN_31_2024, 0), JAN_31_2024);
        assert_eq!(add_years(FEB_29_2024, 0), FEB_29_2024);
    }

    #[test]
    fn add_months_clamps_end_of_month() {
        // Jan 31 + 1 month → Feb 29 (leap year)
        assert_eq!(
            add_months(JAN_31_2024, 1),
            datetime_to_unix(2024, 2, 29, 12, 30, 45)
        );
        // Jan 31 + 1 month → Feb 28 (non-leap year)
        assert_eq!(
            add_months(JAN_31_2023, 1),
            datetime_to_unix(2023, 2, 28, 0, 0, 0)
        );
        // Mar 31 + 1 month → Apr 30
        let mar_31_2024 = datetime_to_unix(2024, 3, 31, 10, 0, 0);
        assert_eq!(
            add_months(mar_31_2024, 1),
            datetime_to_unix(2024, 4, 30, 10, 0, 0)
        );
        // May 31 + 1 month → Jun 30
        let may_31_2024 = datetime_to_unix(2024, 5, 31, 0, 0, 0);
        assert_eq!(
            add_months(may_31_2024, 1),
            datetime_to_unix(2024, 6, 30, 0, 0, 0)
        );
        // Aug 31 + 1 month → Sep 30
        let aug_31_2024 = datetime_to_unix(2024, 8, 31, 0, 0, 0);
        assert_eq!(
            add_months(aug_31_2024, 1),
            datetime_to_unix(2024, 9, 30, 0, 0, 0)
        );
        // Oct 31 + 1 month → Nov 30
        let oct_31_2024 = datetime_to_unix(2024, 10, 31, 0, 0, 0);
        assert_eq!(
            add_months(oct_31_2024, 1),
            datetime_to_unix(2024, 11, 30, 0, 0, 0)
        );
    }

    #[test]
    fn add_months_handles_multiple_and_large_months() {
        assert_eq!(
            add_months(MAR_15_2024, 12),
            datetime_to_unix(2025, 3, 15, 8, 0, 0)
        );
        // 100 years in months = 1200 months
        assert_eq!(
            add_months(MAR_15_2024, 1200),
            datetime_to_unix(2124, 3, 15, 8, 0, 0)
        );
    }

    #[test]
    fn add_years_preserves_date_and_handles_large_durations() {
        assert_eq!(
            add_years(MAR_15_2024, 1),
            datetime_to_unix(2025, 3, 15, 8, 0, 0)
        );
        assert_eq!(
            add_years(MAR_15_2024, 100),
            datetime_to_unix(2124, 3, 15, 8, 0, 0)
        );
    }

    #[test]
    fn add_years_clamps_leap_day() {
        // Feb 29, 2024 + 1 year → Feb 28, 2025
        assert_eq!(
            add_years(FEB_29_2024, 1),
            datetime_to_unix(2025, 2, 28, 12, 30, 45)
        );
        // Feb 29, 2024 + 4 years → Feb 29, 2028
        assert_eq!(
            add_years(FEB_29_2024, 4),
            datetime_to_unix(2028, 2, 29, 12, 30, 45)
        );
    }

    #[test]
    fn is_leap_year_rules() {
        assert!(is_leap_year(2000));
        assert!(is_leap_year(2024));
        assert!(!is_leap_year(1900));
        assert!(!is_leap_year(2100));
        assert!(!is_leap_year(2023));
    }

    #[test]
    #[should_panic(expected = "invalid month: 0")]
    fn days_in_month_invalid_zero() {
        days_in_month(2024, 0);
    }

    #[test]
    #[should_panic(expected = "invalid month: 13")]
    fn days_in_month_invalid_thirteen() {
        days_in_month(2024, 13);
    }

    #[test]
    #[should_panic(expected = "invalid day 32 for month 1 in year 2024")]
    fn datetime_to_unix_invalid_day() {
        datetime_to_unix(2024, 1, 32, 0, 0, 0);
    }

    #[test]
    #[should_panic(expected = "invalid hour: 24")]
    fn datetime_to_unix_invalid_hour() {
        datetime_to_unix(2024, 1, 1, 24, 0, 0);
    }

    #[test]
    #[should_panic(expected = "year overflow in add_years")]
    fn add_years_overflow_panics() {
        add_years(JAN_31_2024, u32::MAX);
    }

    #[test]
    #[should_panic(expected = "year overflow in add_months")]
    fn add_months_overflow_panics() {
        add_months(JAN_31_2024, u32::MAX);
    }
}
