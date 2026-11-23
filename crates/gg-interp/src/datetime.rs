// SPDX-License-Identifier: GPL-3.0-or-later
//! The native `datetime` module: Unix timestamps and UTC
//! calendar dates, in pure Rust — no external crate.
//!
//! `import datetime` gives `datetime.now()`,
//! `datetime.from_timestamp(ts)`, `datetime.to_timestamp(dict)`,
//! `datetime.format(ts_or_dict, fmt)` and
//! `datetime.date(year, month, day)`.
//!
//! Everything is UTC: there is no local time here (that is a
//! later step). A timestamp is an Int, seconds since
//! 1970-01-01 00:00:00 UTC. `from_timestamp` returns a Dict
//! with the keys `year`, `month`, `day`, `hour`, `minute`,
//! `second` and `weekday`, where `weekday` is 0 for Monday
//! through 6 for Sunday (1970-01-01 was a Thursday, so
//! `from_timestamp(0)["weekday"]` is 3). `to_timestamp` is the
//! inverse: it reads the same fields (any `weekday` key is
//! ignored, it is recomputed) and validates them — month 1-12,
//! a day that exists in that month of that year (leap years
//! included), hour 0-23, minute and second 0-59 — anything
//! else is a runtime error. `format` takes either an Int
//! timestamp or a Dict like the one `from_timestamp` returns,
//! and a format string with a `strftime` subset: `%Y` (year),
//! `%m` (month), `%d` (day), `%H` (hour), `%M` (minute), `%S`
//! (second) and `%w` (weekday, 0-6 as above); `%%` is a literal
//! `%`, any other directive is an error. `date` is the
//! midnight-UTC timestamp of a calendar day.
//!
//! The calendar math is Howard Hinnant's civil-from-days /
//! days-from-civil algorithm, written out by hand: it works
//! for negative timestamps too (before 1970), and the internal
//! day count is i128 so no plausible year overflows it — the
//! final timestamp is checked back into i64.

use std::cell::RefCell;
use std::io::{BufRead, Write};
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::RuntimeError;
use crate::value::{DictData, DictKey, Value};

/// Seconds in a day; also the modulus for splitting a
/// timestamp into a day count and a time of day.
const SECONDS_PER_DAY: i64 = 86_400;

/// A broken-down UTC date and time. `weekday` is 0 for Monday
/// through 6 for Sunday.
struct Parts {
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
    weekday: i64,
}

/// Is `year` a leap year? Divisible by 4, except centuries,
/// except centuries divisible by 400.
fn is_leap(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

/// The days in `month` of `year` (month 1-12; anything else is
/// 0, so callers validate the month first).
fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap(year) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

/// Days from 1970-01-01 to the given civil date. The count is
/// i128 so no year a user can type overflows it.
fn days_from_civil(year: i64, month: i64, day: i64) -> i128 {
    let mut y = year as i128;
    if month <= 2 {
        y -= 1;
    }
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = if month > 2 { month - 3 } else { month + 9 } as i128; // [0, 11]
    let doy = (153 * mp + 2) / 5 + day as i128 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The civil date `days` after 1970-01-01, as
/// (year, month, day).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days as i128 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let mut y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    if m <= 2 {
        y += 1;
    }
    (y as i64, m as i64, d as i64)
}

/// The weekday of a day count, 0 for Monday through 6 for
/// Sunday. Day 0 (1970-01-01) was a Thursday, which is 3.
fn weekday_of(days: i128) -> i64 {
    (days + 3).rem_euclid(7) as i64
}

/// Split a timestamp into its UTC parts. Negative timestamps
/// work: `div_euclid`/`rem_euclid` keep the time of day
/// non-negative, so -1 is 1969-12-31 23:59:59.
fn parts_from_timestamp(ts: i64) -> Parts {
    let days = ts.div_euclid(SECONDS_PER_DAY);
    let secs = ts.rem_euclid(SECONDS_PER_DAY);
    let (year, month, day) = civil_from_days(days);
    Parts {
        year,
        month,
        day,
        hour: secs / 3600,
        minute: (secs % 3600) / 60,
        second: secs % 60,
        weekday: weekday_of(days as i128),
    }
}

/// A timestamp from validated parts, or an error when it does
/// not fit in an i64.
fn timestamp_from_parts(name: &str, parts: &Parts) -> Result<i64, RuntimeError> {
    let seconds = days_from_civil(parts.year, parts.month, parts.day) * SECONDS_PER_DAY as i128
        + parts.hour as i128 * 3600
        + parts.minute as i128 * 60
        + parts.second as i128;
    i64::try_from(seconds).map_err(|_| RuntimeError::new(format!("{name}() result out of range")))
}

/// A date is valid when the month is 1-12 and the day exists
/// in that month of that year (leap years included).
fn validate_date(name: &str, year: i64, month: i64, day: i64) -> Result<(), RuntimeError> {
    if !(1..=12).contains(&month) {
        return Err(RuntimeError::new(format!(
            "{name}() month {month} out of range 1-12"
        )));
    }
    let last = days_in_month(year, month);
    if !(1..=last).contains(&day) {
        return Err(RuntimeError::new(format!(
            "{name}() day {day} out of range 1-{last} for {year}-{month:02}"
        )));
    }
    Ok(())
}

/// A time of day is valid when the hour is 0-23 and the minute
/// and second are 0-59.
fn validate_time(name: &str, hour: i64, minute: i64, second: i64) -> Result<(), RuntimeError> {
    if !(0..=23).contains(&hour) {
        return Err(RuntimeError::new(format!(
            "{name}() hour {hour} out of range 0-23"
        )));
    }
    if !(0..=59).contains(&minute) {
        return Err(RuntimeError::new(format!(
            "{name}() minute {minute} out of range 0-59"
        )));
    }
    if !(0..=59).contains(&second) {
        return Err(RuntimeError::new(format!(
            "{name}() second {second} out of range 0-59"
        )));
    }
    Ok(())
}

/// An integer argument, or a clear error naming the function.
fn as_int(name: &str, value: &Value) -> Result<i64, RuntimeError> {
    match value {
        Value::Int(n) => Ok(*n),
        other => Err(RuntimeError::new(format!(
            "{name}() expects an Int, not {}",
            other.type_name()
        ))),
    }
}

/// A Dict as broken-down parts: the `year`, `month`, `day`,
/// `hour`, `minute` and `second` fields, each an Int, all
/// validated. Extra keys (including `weekday`) are ignored:
/// the weekday is recomputed from the date.
fn dict_to_parts(name: &str, value: &Value) -> Result<Parts, RuntimeError> {
    let Value::Dict(dict) = value else {
        return Err(RuntimeError::new(format!(
            "{name}() expects a Dict, not {}",
            value.type_name()
        )));
    };
    let borrowed = dict.borrow();
    let field = |key: &str| -> Result<i64, RuntimeError> {
        match borrowed.get(&DictKey::Str(key.to_string())) {
            None => Err(RuntimeError::new(format!(
                "{name}() missing field \"{key}\""
            ))),
            Some(Value::Int(n)) => Ok(*n),
            Some(other) => Err(RuntimeError::new(format!(
                "{name}() field \"{key}\" must be an Int, not {}",
                other.type_name()
            ))),
        }
    };
    let year = field("year")?;
    let month = field("month")?;
    let day = field("day")?;
    let hour = field("hour")?;
    let minute = field("minute")?;
    let second = field("second")?;
    validate_date(name, year, month, day)?;
    validate_time(name, hour, minute, second)?;
    Ok(Parts {
        year,
        month,
        day,
        hour,
        minute,
        second,
        weekday: weekday_of(days_from_civil(year, month, day)),
    })
}

/// Parts as a GG Dict, in the `from_timestamp` field order.
fn parts_to_dict(parts: &Parts) -> Value {
    let mut dict = DictData::new();
    for (key, value) in [
        ("year", parts.year),
        ("month", parts.month),
        ("day", parts.day),
        ("hour", parts.hour),
        ("minute", parts.minute),
        ("second", parts.second),
        ("weekday", parts.weekday),
    ] {
        dict.insert(DictKey::Str(key.to_string()), Value::Int(value));
    }
    Value::Dict(Rc::new(RefCell::new(dict)))
}

/// `datetime.now()` — the current Unix timestamp in seconds,
/// as an Int.
pub fn now(
    _args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => i64::try_from(duration.as_secs())
            .map(Value::Int)
            .map_err(|_| RuntimeError::new("datetime.now() out of range")),
        Err(_) => Err(RuntimeError::new(
            "datetime.now(): system time is before 1970-01-01",
        )),
    }
}

/// `datetime.from_timestamp(ts)` — the UTC date and time of a
/// Unix timestamp, as a Dict with `year`, `month`, `day`,
/// `hour`, `minute`, `second` and `weekday` (0 = Monday ..
/// 6 = Sunday).
pub fn from_timestamp(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let ts = as_int("datetime.from_timestamp", &args[0])?;
    Ok(parts_to_dict(&parts_from_timestamp(ts)))
}

/// `datetime.to_timestamp(dict)` — the Unix timestamp of a
/// Dict like the one `from_timestamp` returns, validating
/// every field (month 1-12, an existing day, hour 0-23,
/// minute and second 0-59).
pub fn to_timestamp(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let parts = dict_to_parts("datetime.to_timestamp", &args[0])?;
    timestamp_from_parts("datetime.to_timestamp", &parts).map(Value::Int)
}

/// `datetime.format(ts_or_dict, fmt)` — the date and time as a
/// Str, with a `strftime` subset: `%Y` year, `%m` month, `%d`
/// day, `%H` hour, `%M` minute, `%S` second, `%w` weekday
/// (0 = Monday .. 6 = Sunday), `%%` a literal `%`. The first
/// argument is an Int timestamp or a Dict like the one
/// `from_timestamp` returns. Any other directive is an error.
pub fn format(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let parts = match &args[0] {
        Value::Int(ts) => parts_from_timestamp(*ts),
        Value::Dict(_) => dict_to_parts("datetime.format", &args[0])?,
        other => {
            return Err(RuntimeError::new(format!(
                "datetime.format() expects an Int or a Dict, not {}",
                other.type_name()
            )));
        }
    };
    let Value::Str(fmt) = &args[1] else {
        return Err(RuntimeError::new(format!(
            "datetime.format() expects a Str format, not {}",
            args[1].type_name()
        )));
    };
    let mut out = String::new();
    let mut chars = fmt.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            None => {
                return Err(RuntimeError::new(
                    "datetime.format(): trailing % in format string",
                ));
            }
            Some('%') => out.push('%'),
            Some('Y') => out.push_str(&format!("{:04}", parts.year)),
            Some('m') => out.push_str(&format!("{:02}", parts.month)),
            Some('d') => out.push_str(&format!("{:02}", parts.day)),
            Some('H') => out.push_str(&format!("{:02}", parts.hour)),
            Some('M') => out.push_str(&format!("{:02}", parts.minute)),
            Some('S') => out.push_str(&format!("{:02}", parts.second)),
            Some('w') => out.push_str(&parts.weekday.to_string()),
            Some(other) => {
                return Err(RuntimeError::new(format!(
                    "datetime.format(): unknown directive %{other}"
                )));
            }
        }
    }
    Ok(Value::Str(out))
}

/// `datetime.date(year, month, day)` — the Unix timestamp of
/// midnight UTC on that day, validating the date (month 1-12,
/// an existing day, leap years included).
pub fn date(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let year = as_int("datetime.date", &args[0])?;
    let month = as_int("datetime.date", &args[1])?;
    let day = as_int("datetime.date", &args[2])?;
    validate_date("datetime.date", year, month, day)?;
    let parts = Parts {
        year,
        month,
        day,
        hour: 0,
        minute: 0,
        second: 0,
        weekday: weekday_of(days_from_civil(year, month, day)),
    };
    timestamp_from_parts("datetime.date", &parts).map(Value::Int)
}
