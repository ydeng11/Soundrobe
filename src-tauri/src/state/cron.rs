//! Minimal 5-field cron evaluator for the background-tagging schedule.
//!
//! The reference product schedules its background scraping with a crontab
//! expression and offers quick presets (`*/1 * * * *`, `0 0 * * *`, …). The
//! desktop app cannot rely on an external scheduler, so the expression is
//! evaluated in-process. Only the deterministic part lives here: parsing and
//! "when is the next occurrence". Running the job is the caller's concern.
//!
//! Field order matches cron: minute, hour, day-of-month, month, day-of-week.
//! Day-of-week accepts `0` or `7` for Sunday. When both day-of-month and
//! day-of-week are restricted, a date matches if *either* matches, which is
//! the standard cron behaviour and the reason `0 0 13 * 5` fires on every
//! Friday as well as every 13th.

use time::{Date, PrimitiveDateTime, Time, Weekday};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CronError {
    /// The expression did not have exactly five whitespace-separated fields.
    FieldCount(usize),
    /// A field contained something that is neither `*`, a number, nor `-`/`,`.
    BadField {
        field: &'static str,
        value: String,
    },
    /// A field named a number outside its allowed range.
    OutOfRange {
        field: &'static str,
        value: i64,
    },
    /// A `*/0` style step, which can never match anything.
    StepZero(&'static str),
}

impl std::fmt::Display for CronError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FieldCount(count) => write!(
                formatter,
                "cron expression needs 5 fields (minute hour day month weekday), got {count}"
            ),
            Self::BadField { field, value } => {
                write!(formatter, "invalid value {value:?} in cron {field} field")
            }
            Self::OutOfRange { field, value } => {
                write!(formatter, "value {value} is out of range for cron {field} field")
            }
            Self::StepZero(field) => write!(formatter, "cron step of 0 in {field} field"),
        }
    }
}

impl std::error::Error for CronError {}

const FIELD_NAMES: [&str; 5] = ["minute", "hour", "day-of-month", "month", "day-of-week"];
const FIELD_RANGES: [(i64, i64); 5] = [(0, 59), (0, 23), (1, 31), (1, 12), (0, 7)];

/// A parsed crontab expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronSchedule {
    fields: [Vec<u8>; 5],
    /// `true` when the field was written as `*`, which changes how
    /// day-of-month and day-of-week combine.
    unrestricted: [bool; 5],
}

impl CronSchedule {
    /// Parse a `minute hour day month weekday` expression.
    pub fn parse(expression: &str) -> Result<Self, CronError> {
        let parts: Vec<&str> = expression.split_whitespace().collect();
        if parts.len() != 5 {
            return Err(CronError::FieldCount(parts.len()));
        }

        let mut fields: [Vec<u8>; 5] = Default::default();
        let mut unrestricted = [false; 5];

        for (index, raw) in parts.iter().enumerate() {
            let field = FIELD_NAMES[index];
            let (min, max) = FIELD_RANGES[index];
            unrestricted[index] = raw.trim() == "*";

            let mut values: Vec<u8> = Vec::new();
            for chunk in raw.split(',') {
                let chunk = chunk.trim();
                if chunk.is_empty() {
                    return Err(CronError::BadField {
                        field,
                        value: (*raw).to_string(),
                    });
                }

                let (range_part, step) = match chunk.split_once('/') {
                    Some((range, step)) => {
                        let step: i64 = step.trim().parse().map_err(|_| CronError::BadField {
                            field,
                            value: (*raw).to_string(),
                        })?;
                        if step <= 0 {
                            return Err(CronError::StepZero(field));
                        }
                        (range.trim(), step)
                    }
                    None => (chunk, 1),
                };

                let (low, high) = if range_part == "*" {
                    (min, max)
                } else if let Some((start, end)) = range_part.split_once('-') {
                    (
                        parse_number(start, field, raw)?,
                        parse_number(end, field, raw)?,
                    )
                } else {
                    let single = parse_number(range_part, field, raw)?;
                    // A bare value with a step (`5/15`) means "from 5 upward".
                    if step > 1 {
                        (single, max)
                    } else {
                        (single, single)
                    }
                };

                if low < min || low > max {
                    return Err(CronError::OutOfRange {
                        field,
                        value: low,
                    });
                }
                if high < min || high > max {
                    return Err(CronError::OutOfRange {
                        field,
                        value: high,
                    });
                }
                if high < low {
                    return Err(CronError::BadField {
                        field,
                        value: (*raw).to_string(),
                    });
                }

                let mut value = low;
                while value <= high {
                    values.push(value as u8);
                    value += step;
                }
            }

            values.sort_unstable();
            values.dedup();
            if values.is_empty() {
                return Err(CronError::BadField {
                    field,
                    value: (*raw).to_string(),
                });
            }
            if field == "day-of-week" {
                // Cron accepts both 0 and 7 for Sunday; matching uses 0..=6.
                for value in values.iter_mut() {
                    if *value == 7 {
                        *value = 0;
                    }
                }
                values.sort_unstable();
                values.dedup();
            }
            fields[index] = values;
        }

        Ok(Self {
            fields,
            unrestricted,
        })
    }

    /// The first scheduled instant strictly after `from`, to the minute.
    ///
    /// Callers that poll repeatedly should pass the previous evaluation time
    /// rather than "now", so an occurrence is neither skipped nor repeated.
    pub fn next_after(&self, from: PrimitiveDateTime) -> Option<PrimitiveDateTime> {
        // Start at the next whole minute.
        let start = from + time::Duration::seconds(60 - i64::from(from.second()));
        let mut date = start.date();

        // Five years covers the rarest valid cron date (`29 2` in a leap year).
        for _ in 0..(366 * 5) {
            if self.matches_date(date) {
                let first_hour = if date == start.date() {
                    start.hour()
                } else {
                    0
                };
                if let Some(hour) = self.next_value(&self.fields[1], first_hour) {
                    let first_minute = if date == start.date() && hour == start.hour() {
                        start.minute()
                    } else {
                        0
                    };
                    if let Some(minute) = self.next_value(&self.fields[0], first_minute) {
                        return Some(PrimitiveDateTime::new(
                            date,
                            Time::from_hms(hour, minute, 0).ok()?,
                        ));
                    }
                    // A later hour on the same date can still satisfy the minute.
                    if let Some(hour) = self.next_value(&self.fields[1], hour + 1) {
                        if let Some(minute) = self.fields[0].first().copied() {
                            return Some(PrimitiveDateTime::new(
                                date,
                                Time::from_hms(hour, minute, 0).ok()?,
                            ));
                        }
                    }
                }
            }
            date = date.next_day()?;
        }

        None
    }

    /// The first value in `values` that is `>= floor`, if any.
    fn next_value(&self, values: &[u8], floor: u8) -> Option<u8> {
        values.iter().copied().find(|value| *value >= floor)
    }

    fn matches_date(&self, date: Date) -> bool {
        if !self.fields[3].contains(&(date.month() as u8)) {
            return false;
        }

        let day = date.day();
        let weekday = weekday_zero_based(date.weekday());
        let dom_match = self.fields[2].contains(&day);
        let dow_match = self.fields[4].contains(&weekday);

        // Standard cron: `*` on one side defers to the other; two restricted
        // sides are OR-ed.
        match (self.unrestricted[2], self.unrestricted[4]) {
            (true, true) => true,
            (true, false) => dow_match,
            (false, true) => dom_match,
            (false, false) => dom_match || dow_match,
        }
    }
}

/// Cron numbers Sunday as 0 (and 7); `time` numbers it 7 in `number_from_sunday`.
fn weekday_zero_based(weekday: Weekday) -> u8 {
    weekday.number_from_sunday() - 1
}

fn parse_number(raw: &str, field: &'static str, whole: &str) -> Result<i64, CronError> {
    raw.trim().parse::<i64>().map_err(|_| CronError::BadField {
        field,
        value: whole.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::Month;

    fn dt(year: i32, month: u8, day: u8, hour: u8, minute: u8) -> PrimitiveDateTime {
        PrimitiveDateTime::new(
            Date::from_calendar_date(year, Month::try_from(month).unwrap(), day).unwrap(),
            Time::from_hms(hour, minute, 0).unwrap(),
        )
    }

    fn next(expression: &str, from: PrimitiveDateTime) -> PrimitiveDateTime {
        CronSchedule::parse(expression)
            .expect("expression should parse")
            .next_after(from)
            .expect("expression should have a next occurrence")
    }

    /// 2026-09-18 is a Friday; every case below is anchored to it.
    fn friday() -> PrimitiveDateTime {
        dt(2026, 9, 18, 13, 45)
    }

    #[test]
    fn every_minute_advances_one_minute() {
        assert_eq!(next("*/1 * * * *", friday()), dt(2026, 9, 18, 13, 46));
    }

    #[test]
    fn a_fixed_time_rolls_over_to_the_next_day() {
        assert_eq!(next("30 2 * * *", friday()), dt(2026, 9, 19, 2, 30));
        assert_eq!(next("0 0 * * *", friday()), dt(2026, 9, 19, 0, 0));
    }

    #[test]
    fn steps_ranges_and_lists_select_the_expected_slots() {
        assert_eq!(next("*/15 * * * *", friday()), dt(2026, 9, 18, 14, 0));
        assert_eq!(
            next("*/15 * * * *", dt(2026, 9, 18, 10, 20)),
            dt(2026, 9, 18, 10, 30)
        );
        assert_eq!(
            next("15,45 * * * *", dt(2026, 9, 18, 10, 20)),
            dt(2026, 9, 18, 10, 45)
        );
        assert_eq!(
            next("0 */2 * * *", dt(2026, 9, 18, 3, 10)),
            dt(2026, 9, 18, 4, 0)
        );
        assert_eq!(
            next("0 9-17 * * *", dt(2026, 9, 18, 8, 10)),
            dt(2026, 9, 18, 9, 0)
        );
    }

    #[test]
    fn a_poll_that_already_matched_moves_to_the_following_slot() {
        // The scheduler passes its previous evaluation time, so landing exactly
        // on a slot must advance rather than repeat it.
        assert_eq!(next("*/15 * * * *", dt(2026, 9, 18, 10, 30)), dt(2026, 9, 18, 10, 45));
        assert_eq!(next("0 * * * *", dt(2026, 9, 18, 10, 0)), dt(2026, 9, 18, 11, 0));
    }

    #[test]
    fn weekday_names_both_meanings_of_sunday_and_monday() {
        assert_eq!(next("0 0 * * 1", friday()), dt(2026, 9, 21, 0, 0));
        assert_eq!(next("0 0 * * 0", friday()), dt(2026, 9, 20, 0, 0));
        assert_eq!(next("0 0 * * 7", friday()), dt(2026, 9, 20, 0, 0));
        assert_eq!(next("0 0 * * 5", friday()), dt(2026, 9, 25, 0, 0));
    }

    #[test]
    fn two_restricted_day_fields_are_or_ed() {
        // If these were AND-ed, the next run would be a Friday the 13th.
        assert_eq!(next("0 0 13 * 5", friday()), dt(2026, 9, 25, 0, 0));
        assert_eq!(next("0 0 1 * 1", friday()), dt(2026, 9, 21, 0, 0));
    }

    #[test]
    fn restricted_day_of_month_ignores_the_weekday_field() {
        assert_eq!(next("0 0 20 * *", friday()), dt(2026, 9, 20, 0, 0));
    }

    #[test]
    fn yearly_and_leap_day_expressions_reach_their_only_dates() {
        assert_eq!(next("0 0 1 1 *", friday()), dt(2027, 1, 1, 0, 0));
        assert_eq!(next("0 0 29 2 *", friday()), dt(2028, 2, 29, 0, 0));
    }

    #[test]
    fn malformed_expressions_are_rejected_with_a_reason() {
        assert_eq!(CronSchedule::parse(""), Err(CronError::FieldCount(0)));
        assert_eq!(CronSchedule::parse("* * * *"), Err(CronError::FieldCount(4)));
        assert_eq!(CronSchedule::parse("* * * * * *"), Err(CronError::FieldCount(6)));
        assert_eq!(
            CronSchedule::parse("60 * * * *"),
            Err(CronError::OutOfRange {
                field: "minute",
                value: 60
            })
        );
        assert_eq!(
            CronSchedule::parse("* 24 * * *"),
            Err(CronError::OutOfRange {
                field: "hour",
                value: 24
            })
        );
        assert_eq!(CronSchedule::parse("*/0 * * * *"), Err(CronError::StepZero("minute")));
        assert_eq!(
            CronSchedule::parse("abc * * * *"),
            Err(CronError::BadField {
                field: "minute",
                value: "abc".to_string()
            })
        );
        assert_eq!(
            CronSchedule::parse("* * 0 * *"),
            Err(CronError::OutOfRange {
                field: "day-of-month",
                value: 0
            })
        );
    }

    #[test]
    fn duplicate_and_unsorted_values_collapse_to_one_slot() {
        let schedule = CronSchedule::parse("30,30,15 * * * *").unwrap();
        assert_eq!(schedule.fields[0], vec![15, 30]);
    }
}
