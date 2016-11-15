use std::collections::{BTreeSet, HashSet};
use std::iter::FromIterator;
use std::cmp;
use chrono::{DateTime, Date, Datelike, TimeZone, Local, NaiveTime, Weekday, Duration as CDuration};

type TimeSet = BTreeSet<NaiveTime>;
type WeekdaySet = HashSet<Weekday>;

/// Represents the different types of date-time bounds that can be on a schedule
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
    pub fn resolve_from(&self, date: &DateTime<Local>) -> Option<DateTime<Local>> {
        match *self {
            DateTimeBound::None => None,
            DateTimeBound::Yearly(date_time) => date_time.with_year(date.year()),
            DateTimeBound::Definite(date_time) => Some(date_time),
        }
    }

    pub fn resolve(&self) -> Option<DateTime<Local>> {
        self.resolve_from(&Local::now())
    }
}

/// A schedule that determines when an event will occur.
#[derive(Default)]
pub struct Schedule {
    times: TimeSet,
    weekdays: WeekdaySet,
    from: DateTimeBound,
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
    /// # Examples
    /// ```
    /// # extern crate rinklers;
    /// # extern crate chrono;
    /// # fn main() {
    /// # }
    /// ```
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

    pub fn next_run_after(&self, reference: &DateTime<Local>) -> Option<DateTime<Local>> {
        let to = self.to.resolve_from(reference);
        let from = match (self.from.resolve_from(reference), to) {
            (Some(from), Some(to)) if from < to => from.with_year(from.year() + 1),
            (from, _) => from,
        };
        let reference = match from {
            Some(from) if &from > reference => from,
            _ => reference.clone(),
        };
        let mut candidate: Option<DateTime<Local>> = None;
        for weekday in self.weekdays.iter() {
            for time in self.times.iter() {
                let date = next_weekday(&reference.date(), weekday)
                    .and_time(time.clone())
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

    pub fn next_run(&self) -> Option<DateTime<Local>> {
        self.next_run_after(&Local::now())
    }
}

#[cfg(test)]
mod test {
    #[test]
    fn test_program() {
        use super::{Schedule, DateTimeBound};
        use chrono::{NaiveTime, Weekday, Local, TimeZone};
        let schedule = Schedule::new([NaiveTime::from_hms(10, 30, 0)].iter().map(|t| t.clone()),
                                     [Weekday::Wed].iter().map(|t| t.clone()),
                                     DateTimeBound::None,
                                     DateTimeBound::None);
        let date = schedule.next_run_after(&Local.ymd(2016, 11, 14).and_hms(10, 30, 0));
        assert_eq!(date, Some(Local.ymd(2016, 11, 16).and_hms(10, 30, 0)));
    }
}
