//! The one timestamp bitplane writes, and the one it says out loud.
//!
//! ADR-0003 kept a date library out of the contract — timestamps are RFC 3339
//! strings on the wire — and the only thing bitplane *writes* is the incomplete
//! latch, whose contents nothing parses (ADR-0004). So the whole requirement is
//! "render a `SystemTime` as UTC RFC 3339", which is a dozen lines of civil
//! calendar arithmetic and not a dependency.
//!
//! [`ago`] is the second half of that: a finding saying *"create never
//! completed, started 3 days ago"* is the difference between confidently
//! discarding a remnant and wondering whether something is still running, and a
//! bare timestamp does not say it.

use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// An instant, rendered as RFC 3339 in UTC to whole seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Rfc3339 {
    seconds_since_epoch: i64,
}

impl Rfc3339 {
    /// Now.
    pub fn now() -> Rfc3339 {
        Rfc3339::at(SystemTime::now())
    }

    /// A given instant. Times before 1970 are representable; the latch will
    /// never hold one, but a clock that has not been set yet can.
    pub fn at(time: SystemTime) -> Rfc3339 {
        let seconds_since_epoch = match time.duration_since(UNIX_EPOCH) {
            Ok(elapsed) => elapsed.as_secs() as i64,
            Err(before) => -(before.duration().as_secs() as i64),
        };
        Rfc3339 {
            seconds_since_epoch,
        }
    }
}

impl fmt::Display for Rfc3339 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let days = self.seconds_since_epoch.div_euclid(SECONDS_PER_DAY);
        let second_of_day = self.seconds_since_epoch.rem_euclid(SECONDS_PER_DAY);

        let (year, month, day) = civil_from_days(days);
        let (hour, minute, second) = (
            second_of_day / 3600,
            (second_of_day % 3600) / 60,
            second_of_day % 60,
        );

        write!(
            f,
            "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z"
        )
    }
}

/// How long `elapsed` is, in the words a finding prints before "ago".
///
/// Coarsened deliberately to one unit: the reader is deciding whether a remnant
/// is abandoned or still being built, and "3 days" answers that where
/// "3 days, 4 hours and 11 minutes" only makes it harder to read.
pub fn ago(elapsed: Duration) -> String {
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;

    let seconds = elapsed.as_secs();

    if seconds < MINUTE {
        "less than a minute".to_owned()
    } else if seconds < HOUR {
        counted(seconds / MINUTE, "minute")
    } else if seconds < DAY {
        counted(seconds / HOUR, "hour")
    } else {
        counted(seconds / DAY, "day")
    }
}

fn counted(count: u64, unit: &str) -> String {
    if count == 1 {
        format!("1 {unit}")
    } else {
        format!("{count} {unit}s")
    }
}

const SECONDS_PER_DAY: i64 = 86_400;

/// Howard Hinnant's `civil_from_days`, the standard branch-free conversion from
/// a day count since the Unix epoch to a proleptic Gregorian date.
fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    // Shift to an era starting 0000-03-01, so a leap day lands at the end.
    let days = days_since_epoch + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;

    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);

    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_elapsed_span_is_worded_in_its_largest_whole_unit() {
        let cases = [
            (0, "less than a minute"),
            (59, "less than a minute"),
            (60, "1 minute"),
            (119, "1 minute"),
            (120, "2 minutes"),
            (3_600, "1 hour"),
            (7_200, "2 hours"),
            (86_400, "1 day"),
            (259_200, "3 days"),
        ];

        for (seconds, expected) in cases {
            assert_eq!(
                ago(Duration::from_secs(seconds)),
                expected,
                "for {seconds}s"
            );
        }
    }

    #[test]
    fn the_epoch_and_the_dates_around_it_render() {
        let cases = [
            (0, "1970-01-01T00:00:00Z"),
            (1, "1970-01-01T00:00:01Z"),
            (86_399, "1970-01-01T23:59:59Z"),
            (86_400, "1970-01-02T00:00:00Z"),
            // A leap day, and the day after it.
            (951_782_400, "2000-02-29T00:00:00Z"),
            (951_868_800, "2000-03-01T00:00:00Z"),
            // 2100 is not a leap year, which is the case a naive /4 gets wrong.
            (4_107_456_000, "2100-02-28T00:00:00Z"),
            (4_107_542_400, "2100-03-01T00:00:00Z"),
            (1_758_547_392, "2025-09-22T13:23:12Z"),
        ];

        for (seconds, expected) in cases {
            let at = UNIX_EPOCH + Duration::from_secs(seconds);
            assert_eq!(Rfc3339::at(at).to_string(), expected, "for {seconds}");
        }
    }

    #[test]
    fn an_instant_before_the_epoch_still_renders() {
        let at = UNIX_EPOCH - Duration::from_secs(1);

        assert_eq!(Rfc3339::at(at).to_string(), "1969-12-31T23:59:59Z");
    }

    #[test]
    fn now_is_a_date_in_this_century() {
        let now = Rfc3339::now().to_string();

        assert!(now.starts_with("20"), "got {now}");
        assert_eq!(now.len(), 20, "got {now}");
    }
}
