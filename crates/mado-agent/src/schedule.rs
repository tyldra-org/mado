//! Routine schedules (spec 13 §12): the grammar and the next run after a
//! moment, in the Mac's time zone.

use jiff::civil::{Date, Time, Weekday};
use jiff::{ToSpan, Zoned};

/// Which of a month's weekdays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nth {
    First,
    Second,
    Third,
    Fourth,
    Last,
}

/// A parsed schedule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Schedule {
    /// `daily`, `weekdays`, `weekends` and `<mon,tue,...>`: on these days, at
    /// these times.
    Days {
        days: Vec<Weekday>,
        times: Vec<Time>,
    },
    Hourly,
    EveryHours(i64),
    EveryDays {
        n: i64,
        at: Time,
    },
    EveryWeeks {
        n: i64,
        day: Weekday,
        at: Time,
    },
    /// A day of the month; months without it are skipped. `None` is the
    /// last day.
    Monthly {
        day: Option<i8>,
        at: Time,
    },
    NthWeekday {
        nth: Nth,
        day: Weekday,
        at: Time,
    },
    Yearly {
        month: i8,
        day: i8,
        at: Time,
    },
    /// When the profile opens.
    OnOpen,
}

const ALL: [Weekday; 7] = [
    Weekday::Monday,
    Weekday::Tuesday,
    Weekday::Wednesday,
    Weekday::Thursday,
    Weekday::Friday,
    Weekday::Saturday,
    Weekday::Sunday,
];

fn weekday(s: &str) -> Option<Weekday> {
    let s = s.to_lowercase();
    let i = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"]
        .iter()
        .position(|d| s.starts_with(d))?;
    Some(ALL[i])
}

fn month(s: &str) -> Option<i8> {
    let s = s.to_lowercase();
    let i = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ]
    .iter()
    .position(|m| s.starts_with(m))?;
    i8::try_from(i + 1).ok()
}

fn time(s: &str) -> Result<Time, String> {
    let (h, m) = s
        .split_once(':')
        .ok_or_else(|| format!("{s} is not HH:MM"))?;
    let h: i8 = h.parse().map_err(|_| format!("{s} is not HH:MM"))?;
    let m: i8 = m.parse().map_err(|_| format!("{s} is not HH:MM"))?;
    Time::new(h, m, 0, 0).map_err(|_| format!("{s} is not a time"))
}

fn times(s: &str) -> Result<Vec<Time>, String> {
    let mut t: Vec<Time> = s
        .split(',')
        .map(|p| time(p.trim()))
        .collect::<Result<_, _>>()?;
    t.sort();
    t.dedup();
    Ok(t)
}

fn positive(s: &str) -> Result<i64, String> {
    s.parse::<i64>()
        .ok()
        .filter(|n| (1..=1000).contains(n))
        .ok_or_else(|| format!("{s} is not a number from 1"))
}

/// Parses the grammar of 13 §12.
pub fn parse(text: &str) -> Result<Schedule, String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let lower: Vec<String> = words.iter().map(|w| w.to_lowercase()).collect();
    let w: Vec<&str> = lower.iter().map(String::as_str).collect();
    let days = |d: &[Weekday], t: &str| {
        Ok(Schedule::Days {
            days: d.to_vec(),
            times: times(t)?,
        })
    };
    match w.as_slice() {
        ["on", "open"] => Ok(Schedule::OnOpen),
        ["hourly"] => Ok(Schedule::Hourly),
        ["daily", t] => days(&ALL, t),
        ["weekdays", t] => days(&ALL[..5], t),
        ["weekends", t] => days(&ALL[5..], t),
        ["every", n, "days" | "day", t] => Ok(Schedule::EveryDays {
            n: positive(n)?,
            at: time(t)?,
        }),
        ["every", n, "weeks" | "week", d, t] => Ok(Schedule::EveryWeeks {
            n: positive(n)?,
            day: weekday(d).ok_or_else(|| format!("{d} is not a weekday"))?,
            at: time(t)?,
        }),
        ["every", n] if n.ends_with('h') => Ok(Schedule::EveryHours(positive(&n[..n.len() - 1])?)),
        ["every", n, "h" | "hours" | "hour"] => Ok(Schedule::EveryHours(positive(n)?)),
        ["monthly", "last", t] => Ok(Schedule::Monthly {
            day: None,
            at: time(t)?,
        }),
        ["monthly", d, t] => {
            let d: i8 = d
                .parse()
                .ok()
                .filter(|d| (1..=31).contains(d))
                .ok_or("the day is 1 to 31 or last")?;
            Ok(Schedule::Monthly {
                day: Some(d),
                at: time(t)?,
            })
        }
        ["yearly", m, d, t] => {
            let month = month(m).ok_or_else(|| format!("{m} is not a month"))?;
            let day: i8 = d
                .parse()
                .ok()
                .filter(|d| (1..=31).contains(d))
                .ok_or("the day is 1 to 31")?;
            Date::new(2024, month, day).map_err(|_| format!("{m} has no day {day}"))?;
            Ok(Schedule::Yearly {
                month,
                day,
                at: time(t)?,
            })
        }
        [n @ ("first" | "second" | "third" | "fourth" | "last"), d, t] => {
            Ok(Schedule::NthWeekday {
                nth: match *n {
                    "first" => Nth::First,
                    "second" => Nth::Second,
                    "third" => Nth::Third,
                    "fourth" => Nth::Fourth,
                    _ => Nth::Last,
                },
                day: weekday(d).ok_or_else(|| format!("{d} is not a weekday"))?,
                at: time(t)?,
            })
        }
        [d, t] => {
            let list: Option<Vec<Weekday>> = d.split(',').map(weekday).collect();
            match list {
                Some(l) if !l.is_empty() => days(&l, t),
                _ => Err(format!("{text} is not a schedule")),
            }
        }
        _ => Err(format!("{text} is not a schedule")),
    }
}

fn at(date: Date, t: Time, tz: &jiff::tz::TimeZone) -> Option<Zoned> {
    // A time skipped by daylight saving runs at the next valid moment.
    date.to_datetime(t).to_zoned(tz.clone()).ok()
}

fn nth_weekday(year: i16, month: i8, nth: Nth, day: Weekday) -> Option<Date> {
    let first = Date::new(year, month, 1).ok()?;
    let offset = (day.to_monday_zero_offset() - first.weekday().to_monday_zero_offset() + 7) % 7;
    let d = first.checked_add(i64::from(offset).days()).ok()?;
    let k = match nth {
        Nth::First => 0,
        Nth::Second => 1,
        Nth::Third => 2,
        Nth::Fourth => 3,
        Nth::Last => {
            let mut last = d;
            while let Ok(next) = last.checked_add(7.days()) {
                if next.month() != month {
                    break;
                }
                last = next;
            }
            return Some(last);
        }
    };
    d.checked_add((7 * k).days())
        .ok()
        .filter(|x| x.month() == month)
}

impl Schedule {
    /// The first run strictly after `after`. `anchor` (the routine's
    /// creation) sets the phase of `every` schedules. `None` for `on open`.
    pub fn next(&self, after: &Zoned, anchor: &Zoned) -> Option<Zoned> {
        let tz = after.time_zone();
        let today = after.date();
        // Days from today on, up to eight years for yearly schedules.
        let candidates = |f: &dyn Fn(Date) -> Vec<Time>| -> Option<Zoned> {
            let mut d = today;
            for _ in 0..(366 * 8) {
                for t in f(d) {
                    if let Some(z) = at(d, t, tz)
                        && z > *after
                    {
                        return Some(z);
                    }
                }
                d = d.tomorrow().ok()?;
            }
            None
        };
        match self {
            Self::OnOpen => None,
            Self::Hourly => Self::EveryHours(1).next(after, anchor),
            Self::EveryHours(n) => {
                let step = n * 3600;
                let since = after.timestamp().as_second() - anchor.timestamp().as_second();
                let k = since.div_euclid(step) + 1;
                anchor.checked_add((k * step).seconds()).ok()
            }
            Self::Days { days, times } => candidates(&|d| {
                if days.contains(&d.weekday()) {
                    times.clone()
                } else {
                    Vec::new()
                }
            }),
            Self::EveryDays { n, at: t } => {
                let start = anchor.date();
                candidates(&|d| {
                    let gap = (d - start).get_days();
                    if i64::from(gap).rem_euclid(*n) == 0 {
                        vec![*t]
                    } else {
                        Vec::new()
                    }
                })
            }
            Self::EveryWeeks { n, day, at: t } => {
                let week = |d: Date| {
                    let monday = d
                        .checked_sub(i64::from(d.weekday().to_monday_zero_offset()).days())
                        .unwrap_or(d);
                    i64::from((monday - Date::new(1970, 1, 5).unwrap_or(monday)).get_days()) / 7
                };
                let start = week(anchor.date());
                candidates(&|d| {
                    if d.weekday() == *day && (week(d) - start).rem_euclid(*n) == 0 {
                        vec![*t]
                    } else {
                        Vec::new()
                    }
                })
            }
            Self::Monthly { day, at: t } => candidates(&|d| {
                let hit = match day {
                    Some(n) => d.day() == *n,
                    None => d.day() == d.days_in_month(),
                };
                if hit { vec![*t] } else { Vec::new() }
            }),
            Self::NthWeekday { nth, day, at: t } => candidates(&|d| {
                if nth_weekday(d.year(), d.month(), *nth, *day) == Some(d) {
                    vec![*t]
                } else {
                    Vec::new()
                }
            }),
            Self::Yearly { month, day, at: t } => candidates(&|d| {
                if d.month() == *month && d.day() == *day {
                    vec![*t]
                } else {
                    Vec::new()
                }
            }),
        }
    }
}

/// 13 §12: a run missed while Mado was closed runs once at launch, if the
/// launch is on the missed run's calendar day.
pub fn run_missed(missed: &Zoned, now: &Zoned) -> bool {
    missed <= now && missed.date() == now.date()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn z(s: &str) -> Zoned {
        format!("{s}[Europe/Berlin]").parse().expect("a zoned time")
    }

    fn next(s: &str, after: &str) -> String {
        let a = z(after);
        parse(s)
            .expect(s)
            .next(&a, &z("2026-01-05T08:00"))
            .map(|n| n.datetime().to_string())
            .unwrap_or_default()
    }

    #[test]
    fn the_grammar_parses_and_rejects() {
        for ok in [
            "daily 09:00",
            "daily 09:00,18:30",
            "weekdays 07:15",
            "weekends 10:00",
            "mon,wed,fri 12:00",
            "hourly",
            "every 3h",
            "every 2 days 08:00",
            "every 2 weeks tue 09:00",
            "monthly 31 09:00",
            "monthly last 09:00",
            "first mon 09:00",
            "last fri 17:00",
            "yearly feb 29 09:00",
            "on open",
        ] {
            assert!(parse(ok).is_ok(), "{ok}");
        }
        for bad in [
            "daily",
            "daily 25:00",
            "every 0h",
            "monthly 32 09:00",
            "yearly feb 30 09:00",
            "fifth mon 09:00",
            "sometimes",
            "",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn next_runs() {
        // 2026-01-05 is a Monday.
        assert_eq!(
            next("daily 09:00", "2026-01-05T08:00"),
            "2026-01-05T09:00:00"
        );
        assert_eq!(
            next("daily 09:00,18:30", "2026-01-05T09:00"),
            "2026-01-05T18:30:00"
        );
        assert_eq!(
            next("weekdays 07:15", "2026-01-09T08:00"),
            "2026-01-12T07:15:00"
        );
        assert_eq!(
            next("weekends 10:00", "2026-01-05T08:00"),
            "2026-01-10T10:00:00"
        );
        assert_eq!(
            next("tue,thu 12:00", "2026-01-06T12:00"),
            "2026-01-08T12:00:00"
        );
        assert_eq!(next("hourly", "2026-01-05T10:30"), "2026-01-05T11:00:00");
        assert_eq!(next("every 3h", "2026-01-05T10:30"), "2026-01-05T11:00:00");
        assert_eq!(
            next("every 2 days 08:00", "2026-01-05T09:00"),
            "2026-01-07T08:00:00"
        );
        assert_eq!(
            next("every 2 weeks mon 09:00", "2026-01-05T10:00"),
            "2026-01-19T09:00:00"
        );
        assert_eq!(
            next("monthly 31 09:00", "2026-02-01T00:00"),
            "2026-03-31T09:00:00"
        );
        assert_eq!(
            next("monthly last 09:00", "2026-02-01T00:00"),
            "2026-02-28T09:00:00"
        );
        assert_eq!(
            next("first mon 09:00", "2026-01-05T10:00"),
            "2026-02-02T09:00:00"
        );
        assert_eq!(
            next("last fri 17:00", "2026-01-05T10:00"),
            "2026-01-30T17:00:00"
        );
        assert_eq!(
            next("yearly feb 29 09:00", "2026-01-05T10:00"),
            "2028-02-29T09:00:00"
        );
        assert_eq!(next("on open", "2026-01-05T10:00"), "");
    }

    #[test]
    fn a_time_skipped_by_daylight_saving_runs_after_the_change() {
        // Berlin skips 02:00 to 03:00 on 2026-03-29.
        assert_eq!(
            next("daily 02:30", "2026-03-29T00:00"),
            "2026-03-29T03:30:00"
        );
    }

    #[test]
    fn a_missed_run_runs_at_launch_only_the_same_day() {
        // 13 §13 check 6: 09:00 missed, launch at 10:00 the same day.
        assert!(run_missed(&z("2026-01-05T09:00"), &z("2026-01-05T10:00")));
        assert!(!run_missed(&z("2026-01-04T09:00"), &z("2026-01-05T10:00")));
        assert!(!run_missed(&z("2026-01-05T11:00"), &z("2026-01-05T10:00")));
    }
}
