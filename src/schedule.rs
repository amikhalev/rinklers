//! Scheduling for events to run at certain intervals in the future

use std::collections::{BTreeSet, HashSet};
use std::iter::FromIterator;
use std::cmp;
use chrono::{DateTime, Date, Datelike, TimeZone, Local, NaiveTime, Weekday, Duration as CDuration};

/// A set of times of day (for [Schedule](struct.Schedule.html))
pub type TimeSet = BTreeSet<NaiveTime>;
/// A set of days of week (for [Schedule](struct.Schedule.html))
pub type WeekdaySet = HashSet<Weekday>;

/// Returns a [`WeekdaySet`](type.WeekdaySet.html) of every day of the week
pub fn every_day() -> WeekdaySet {
    use num::FromPrimitive;
    (0..7)
        .map(|i| Weekday::from_u32(i).unwrap())
        .collect()
}

/// Represents the different types of date-time bounds that can be on a schedule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DateTimeBound {
    /// There is no bound (ie. the Schedule extends with no limit)
    None,
    /// There is a bound that repeats every year (ie. the year is set to the current year)
    Yearly(DateTime<Local>),
    /// There is a definite bound on the schedule
    Definite(DateTime<Local>),
}

impl Default for DateTimeBound {
    fn default() -> DateTimeBound {
        DateTimeBound::None
    }
}

impl DateTimeBound {
    /// Resolves this bound into an optional `DateTime`. If there is no bound or the bound could
    /// not be resolved, None is returned.
    ///
    /// `date` is the reference that is used to resolve a `Yearly` bound.
    pub fn resolve_from(&self, date: &DateTime<Local>) -> Option<DateTime<Local>> {
        match *self {
            DateTimeBound::None => None,
            DateTimeBound::Yearly(date_time) => date_time.with_year(date.year()),
            DateTimeBound::Definite(date_time) => Some(date_time),
        }
    }

    /// Resolves this bound with the current date and time
    ///
    /// See [resolve_from](#method.resolve_from)
    pub fn resolve(&self) -> Option<DateTime<Local>> {
        self.resolve_from(&Local::now())
    }
}

/// A schedule that determines when an event will occur.
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct Schedule {
    times: TimeSet,
    weekdays: WeekdaySet,
    #[serde(default)]
    from: DateTimeBound,
    #[serde(default)]
    to: DateTimeBound,
}

/// Gets the next date matching the `weekday` after `date`
fn next_weekday<Tz: TimeZone>(date: &Date<Tz>, weekday: &Weekday) -> Date<Tz> {
    let mut date = date.clone();
    while &date.weekday() != weekday {
        date = date.succ();
    }
    date
}

impl Schedule {
    /// Creates a new Schedule.
    ///
    /// `times` is the times of day the event will be run. `weekdays` is the set of days of week
    /// the event will be run. `from` and `to` are restrictions on the end and beginning of event
    /// runs, respectively.
    pub fn new<T, W>(times: T, weekdays: W, from: DateTimeBound, to: DateTimeBound) -> Schedule
        where T: IntoIterator<Item = NaiveTime>,
              W: IntoIterator<Item = Weekday>
    {
        Schedule {
            times: TimeSet::from_iter(times),
            weekdays: WeekdaySet::from_iter(weekdays),
            from: from,
            to: to,
        }
    }

    /// Gets the next `DateTime` the event should run after `reference`
    ///
    /// Returns `None` if the event will never run after `reference` (ie. must be a `from` bound)
    pub fn next_run_after(&self, reference: &DateTime<Local>) -> Option<DateTime<Local>> {
        let to = self.to.resolve_from(reference);
        let from = match (self.from.resolve_from(reference), to) {
            (Some(from), Some(to)) if from < to => from.with_year(from.year() + 1),
            (from, _) => from,
        };
        let reference = match from {
            Some(from) if &from > reference => from,
            _ => *reference,
        };
        let mut candidate: Option<DateTime<Local>> = None;
        for weekday in &self.weekdays {
            for time in &self.times {
                let date = next_weekday(&reference.date(), weekday)
                    .and_time(*time)
                    .map(|date| if date < reference {
                        date + CDuration::weeks(1)
                    } else {
                        date
                    });
                let date = match (date, to) {
                    (Some(date), Some(to)) if date > to => None,
                    (date, _) => date,
                };
                candidate = match (candidate, date) {
                    // return whichever is first if there are 2 candidates
                    (Some(d1), Some(d2)) => Some(cmp::min(d1, d2)),
                    // otherwise return whichever isn't None (or None if both are)
                    (o1, o2) => o1.or(o2),
                }
            }
        }
        candidate
    }

    /// Gets the next run after the current time
    ///
    /// See [next_run_after](#method.next_run_after)
    pub fn next_run(&self) -> Option<DateTime<Local>> {
        self.next_run_after(&Local::now())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_date_time_bound() {
        use super::DateTimeBound::*;
        use chrono::{DateTime, Local, TimeZone};

        let cases: Vec<(DateTimeBound, Option<DateTime<Local>>)> = vec![(None, Option::None),
                 (Definite(Local.ymd(2016, 11, 16).and_hms(10, 30, 0)),
                  Some(Local.ymd(2016, 11, 16).and_hms(10, 30, 0))),
                 (Yearly(Local.ymd(2016, 11, 16).and_hms(10, 30, 0)),
                  Some(Local.ymd(2018, 11, 16).and_hms(10, 30, 0))),
                 (Yearly(Local.ymd(2016, 1, 1).and_hms(0, 0, 0)),
                  Some(Local.ymd(2018, 1, 1).and_hms(0, 0, 0))),
                 (Yearly(Local.ymd(2016, 12, 31).and_hms(23, 59, 59)),
                  Some(Local.ymd(2018, 12, 31).and_hms(23, 59, 59))),
                 (Yearly(Local.ymd(2012, 2, 29).and_hms(0, 0, 0)), Option::None) /* leap day */];
        let from = Local.ymd(2018, 1, 1).and_hms(0, 0, 0);

        for (bound, expected_result) in cases {
            let result = bound.resolve_from(&from);
            assert_eq!(result, expected_result);
        }
    }

    #[test]
    fn test_next_weekday() {
        use super::next_weekday;
        use chrono::{Date, Local, TimeZone, Weekday};
        // (date, weekday, result)
        let cases: Vec<(Date<Local>, Weekday, Date<Local>)> =
            vec![(Local.ymd(2016, 11, 16), Weekday::Wed, Local.ymd(2016, 11, 16)),
                 (Local.ymd(2016, 11, 16), Weekday::Fri, Local.ymd(2016, 11, 18)),
                 (Local.ymd(2016, 11, 16), Weekday::Tue, Local.ymd(2016, 11, 22)),
                 (Local.ymd(2016, 12, 30), Weekday::Tue, Local.ymd(2017, 1, 3)),
                 (Local.ymd(2016, 11, 16), Weekday::Tue, Local.ymd(2016, 11, 22))];

        for (date, weekday, expected_result) in cases {
            let result = next_weekday(&date, &weekday);
            assert_eq!(result, expected_result);
        }
    }

    #[test]
    fn test_schedule() {
        use super::{Schedule, DateTimeBound};
        use chrono::{DateTime, NaiveTime, Weekday, Local, TimeZone};
        let schedule = Schedule::new(vec![NaiveTime::from_hms(10, 30, 0)],
                                     vec![Weekday::Wed],
                                     DateTimeBound::None,
                                     DateTimeBound::None);
        let cases: Vec<(DateTime<Local>, Option<DateTime<Local>>)> =
            vec![(Local.ymd(2016, 11, 14).and_hms(10, 30, 0),
                  Some(Local.ymd(2016, 11, 16).and_hms(10, 30, 0))),
                 (Local.ymd(2016, 11, 16).and_hms(10, 20, 0),
                  Some(Local.ymd(2016, 11, 16).and_hms(10, 30, 0))),
                 (Local.ymd(2016, 11, 16).and_hms(10, 40, 0),
                  Some(Local.ymd(2016, 11, 23).and_hms(10, 30, 0)))];
        for (reference, expected_result) in cases {
            let result = schedule.next_run_after(&reference);
            assert_eq!(result, expected_result);
        }
    }
}
