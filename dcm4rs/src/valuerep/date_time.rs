//! VRs `DA`, `TM` and `DT`: dates, times and date-times that keep the text they came from.
//!
//! The shapes follow PS3.5 6.2 and pydicom's `DA`, `TM` and `DT`, with two deliberate choices:
//!
//! - A second of `60` is a leap second and PS3.5 allows it. pydicom clamps it to `59` because
//!   Python's `datetime` cannot hold it; here it is kept, text and all.
//! - A fraction of a second needs the seconds before it. pydicom accepts `2001.5` and ignores
//!   the fraction; this rejects it, because accepting and silently dropping it is worse than
//!   saying so.

use std::fmt;
use std::hash::{Hash, Hasher};

use crate::error::{Error, Result};

/// Longest value PS3.5 6.2 allows for VR `DT`, including the UTC offset.
const MAX_DATE_TIME_LENGTH: usize = 26;

/// The date part, shared by `DA` and `DT`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct DateFields {
    year: u16,
    month: u8,
    day: u8,
}

/// The time part, shared by `TM` and `DT`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct TimeFields {
    hour: u8,
    minute: u8,
    second: u8,
    microsecond: u32,
}

/// A date (`DA`): `YYYYMMDD`, or the dotted `YYYY.MM.DD` old ACR-NEMA files used.
#[derive(Clone, Debug)]
pub struct Date {
    fields: DateFields,
    text: String,
}

impl Date {
    /// Parse a `DA` value.
    pub fn parse(text: &str) -> Result<Self> {
        let text = text.trim();
        let fields = parse_date_fields(text)?;
        Ok(Self {
            fields,
            text: text.to_owned(),
        })
    }

    /// Build a date from its parts, written as `YYYYMMDD`.
    pub fn from_ymd(year: u16, month: u8, day: u8) -> Result<Self> {
        let fields = DateFields { year, month, day };
        check_date(fields)?;
        Ok(Self {
            fields,
            text: format!("{year:04}{month:02}{day:02}"),
        })
    }

    /// The year, `0001` to `9999`.
    pub fn year(&self) -> u16 {
        self.fields.year
    }

    /// The month, `1` to `12`.
    pub fn month(&self) -> u8 {
        self.fields.month
    }

    /// The day of the month.
    pub fn day(&self) -> u8 {
        self.fields.day
    }

    /// The characters the value was written as.
    pub fn text(&self) -> &str {
        &self.text
    }
}

/// A time (`TM`): `HH`, `HHMM` or `HHMMSS`, optionally followed by `.FFFFFF` and a space.
#[derive(Clone, Debug)]
pub struct Time {
    fields: TimeFields,
    text: String,
}

impl Time {
    /// Parse a `TM` value.
    pub fn parse(text: &str) -> Result<Self> {
        let text = text.trim();
        let fields = parse_time_fields(text)?;
        Ok(Self {
            fields,
            text: text.to_owned(),
        })
    }

    /// Build a time from hours, minutes and seconds, written as `HHMMSS`. A value without a
    /// fraction is the shape PS3.5 prefers.
    pub fn from_hms(hour: u8, minute: u8, second: u8) -> Result<Self> {
        Self::from_fields(hour, minute, second, 0)
    }

    /// Build a time including microseconds, written as `HHMMSS.FFFFFF` when they are non-zero.
    pub fn from_fields(hour: u8, minute: u8, second: u8, microsecond: u32) -> Result<Self> {
        let fields = TimeFields {
            hour,
            minute,
            second,
            microsecond,
        };
        check_time(fields)?;
        let mut text = format!("{hour:02}{minute:02}{second:02}");
        if microsecond > 0 {
            text.push_str(&format!(".{microsecond:06}"));
        }
        Ok(Self { fields, text })
    }

    /// The hour, `0` to `23`.
    pub fn hour(&self) -> u8 {
        self.fields.hour
    }

    /// The minute, `0` to `59`.
    pub fn minute(&self) -> u8 {
        self.fields.minute
    }

    /// The second, `0` to `60`; `60` is the leap second PS3.5 allows.
    pub fn second(&self) -> u8 {
        self.fields.second
    }

    /// The fraction of a second in microseconds.
    pub fn microsecond(&self) -> u32 {
        self.fields.microsecond
    }

    /// The characters the value was written as.
    pub fn text(&self) -> &str {
        &self.text
    }
}

/// A date-time (`DT`): `YYYY[MM[DD[HH[MM[SS[.FFFFFF]]]]]]`, optionally followed by a UTC offset
/// such as `-0630`.
#[derive(Clone, Debug)]
pub struct DateTime {
    date: DateFields,
    time: Option<TimeFields>,
    offset: Option<i16>,
    text: String,
}

impl DateTime {
    /// Parse a `DT` value.
    ///
    /// `DT` allows the whole range from a bare year to a full timestamp, so the missing parts
    /// take the defaults the standard implies: month and day become `1`, the time becomes
    /// midnight.
    pub fn parse(text: &str) -> Result<Self> {
        let text = text.trim();
        if text.len() > MAX_DATE_TIME_LENGTH {
            return Err(Error::InvalidValue(format!(
                "a date-time has at most {MAX_DATE_TIME_LENGTH} characters: {text}"
            )));
        }

        let (body, offset) = split_offset(text)?;
        let (digits, fraction) = match body.split_once('.') {
            Some((digits, fraction)) => (digits, Some(fraction)),
            None => (body, None),
        };
        let bytes = digits.as_bytes();
        if !(4..=14).contains(&bytes.len()) || bytes.len() % 2 != 0 || !is_digits(bytes) {
            return Err(Error::InvalidValue(format!("not a date-time: {text}")));
        }

        let length = bytes.len();
        let date = DateFields {
            year: number(digits, 0, 4),
            month: if length >= 6 {
                number(digits, 4, 2) as u8
            } else {
                1
            },
            day: if length >= 8 {
                number(digits, 6, 2) as u8
            } else {
                1
            },
        };
        check_date(date)?;

        let time = if length >= 10 {
            let fields = TimeFields {
                hour: number(digits, 8, 2) as u8,
                minute: if length >= 12 {
                    number(digits, 10, 2) as u8
                } else {
                    0
                },
                second: if length >= 14 {
                    number(digits, 12, 2) as u8
                } else {
                    0
                },
                microsecond: fraction_microseconds(fraction)?,
            };
            check_time(fields)?;
            Some(fields)
        } else {
            if fraction.is_some() {
                return Err(Error::InvalidValue(format!(
                    "a date-time fraction needs the seconds before it: {text}"
                )));
            }
            None
        };

        Ok(Self {
            date,
            time,
            offset,
            text: text.to_owned(),
        })
    }

    /// Build a timestamp from its parts, written as `YYYYMMDDHHMMSS`.
    pub fn from_ymd_hms(
        year: u16,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
    ) -> Result<Self> {
        let date = DateFields { year, month, day };
        check_date(date)?;
        let time = TimeFields {
            hour,
            minute,
            second,
            microsecond: 0,
        };
        check_time(time)?;
        Ok(Self {
            date,
            time: Some(time),
            offset: None,
            text: format!("{year:04}{month:02}{day:02}{hour:02}{minute:02}{second:02}"),
        })
    }

    /// The year, `0001` to `9999`.
    pub fn year(&self) -> u16 {
        self.date.year
    }

    /// The month, `1` when the value carries only a year.
    pub fn month(&self) -> u8 {
        self.date.month
    }

    /// The day, `1` when the value carries only a year or a year and month.
    pub fn day(&self) -> u8 {
        self.date.day
    }

    /// The hour, `0` when the value carries only a date.
    pub fn hour(&self) -> u8 {
        self.time_fields().hour
    }

    /// The minute, `0` when the value stops short of minutes.
    pub fn minute(&self) -> u8 {
        self.time_fields().minute
    }

    /// The second, `0` when the value stops short of seconds.
    pub fn second(&self) -> u8 {
        self.time_fields().second
    }

    /// The fraction of a second in microseconds.
    pub fn microsecond(&self) -> u32 {
        self.time_fields().microsecond
    }

    /// Whether the value carries a time of day at all.
    pub fn has_time(&self) -> bool {
        self.time.is_some()
    }

    /// The UTC offset in minutes, negative west of Greenwich, when the value carries one.
    pub fn offset_minutes(&self) -> Option<i16> {
        self.offset
    }

    /// The characters the value was written as.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The time, with the zeros the standard implies when the value has none.
    fn time_fields(&self) -> TimeFields {
        self.time.unwrap_or_default()
    }
}

impl PartialEq for Date {
    fn eq(&self, other: &Self) -> bool {
        self.fields == other.fields
    }
}

impl Eq for Date {}

impl Hash for Date {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.fields.hash(state);
    }
}

impl PartialOrd for Date {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Date {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.fields.cmp(&other.fields)
    }
}

impl PartialEq for Time {
    fn eq(&self, other: &Self) -> bool {
        self.fields == other.fields
    }
}

impl Eq for Time {}

impl Hash for Time {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.fields.hash(state);
    }
}

impl PartialOrd for Time {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Time {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.fields.cmp(&other.fields)
    }
}

/// Two date-times are equal when they name the same instant fields, so `20220101` equals
/// `20220101000000` and the original text does not take part in either equality or order.
///
/// The UTC offset is deliberately left out of both: a value with an offset names a different
/// instant from the same fields without one, and comparing them silently would hide that.
/// Use [`DateTime::offset_minutes`] to compare instants explicitly.
impl PartialEq for DateTime {
    fn eq(&self, other: &Self) -> bool {
        self.date == other.date && self.time_fields() == other.time_fields()
    }
}

impl Eq for DateTime {}

impl Hash for DateTime {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.date.hash(state);
        self.time_fields().hash(state);
    }
}

impl PartialOrd for DateTime {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DateTime {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.date, self.time_fields()).cmp(&(other.date, other.time_fields()))
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl fmt::Display for Time {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl fmt::Display for DateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl std::str::FromStr for Date {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self> {
        Self::parse(text)
    }
}

impl std::str::FromStr for Time {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self> {
        Self::parse(text)
    }
}

impl std::str::FromStr for DateTime {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self> {
        Self::parse(text)
    }
}

/// Parse `YYYYMMDD`, or `YYYY.MM.DD` for the ACR-NEMA 300 spelling.
fn parse_date_fields(text: &str) -> Result<DateFields> {
    let bytes = text.as_bytes();
    let fields = if bytes.len() == 8 && is_digits(bytes) {
        DateFields {
            year: number(text, 0, 4),
            month: number(text, 4, 2) as u8,
            day: number(text, 6, 2) as u8,
        }
    } else if bytes.len() == 10
        && bytes[4] == b'.'
        && bytes[7] == b'.'
        && is_digits(&bytes[0..4])
        && is_digits(&bytes[5..7])
        && is_digits(&bytes[8..10])
    {
        DateFields {
            year: number(text, 0, 4),
            month: number(text, 5, 2) as u8,
            day: number(text, 8, 2) as u8,
        }
    } else {
        return Err(Error::InvalidValue(format!("not a date: {text}")));
    };
    check_date(fields)?;
    Ok(fields)
}

/// Parse `HH`, `HHMM` or `HHMMSS` with an optional fraction of one to six digits.
fn parse_time_fields(text: &str) -> Result<TimeFields> {
    let (main, fraction) = match text.split_once('.') {
        Some((main, fraction)) => (main, Some(fraction)),
        None => (text, None),
    };
    let bytes = main.as_bytes();
    if !matches!(bytes.len(), 2 | 4 | 6) || !is_digits(bytes) {
        return Err(Error::InvalidValue(format!("not a time: {text}")));
    }
    let fields = TimeFields {
        hour: number(main, 0, 2) as u8,
        minute: if bytes.len() >= 4 {
            number(main, 2, 2) as u8
        } else {
            0
        },
        second: if bytes.len() >= 6 {
            number(main, 4, 2) as u8
        } else {
            0
        },
        microsecond: fraction_microseconds(fraction)?,
    };
    check_time(fields)?;
    Ok(fields)
}

/// Convert a fraction of a second to microseconds, padding short fractions on the right, which
/// is what `.1` means: a tenth of a second.
fn fraction_microseconds(fraction: Option<&str>) -> Result<u32> {
    let Some(fraction) = fraction else {
        return Ok(0);
    };
    if fraction.is_empty()
        || fraction.len() > 6
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(Error::InvalidValue(format!(
            "a fraction of a second has one to six digits: .{fraction}"
        )));
    }
    let padded = format!("{fraction:0<6}");
    padded
        .parse()
        .map_err(|_| Error::InvalidValue(format!("not a fraction of a second: .{fraction}")))
}

/// Split the UTC offset suffix `+HHMM` or `-HHMM` off the value and return it in minutes.
fn split_offset(text: &str) -> Result<(&str, Option<i16>)> {
    let bytes = text.as_bytes();
    if bytes.len() < 5 {
        return Ok((text, None));
    }
    let position = bytes.len() - 5;
    if !matches!(bytes[position], b'+' | b'-') || !is_digits(&bytes[position + 1..]) {
        return Ok((text, None));
    }
    let offset = &text[position + 1..];
    let hours = number(offset, 0, 2) as i16;
    let minutes = number(offset, 2, 2) as i16;
    if hours > 23 || minutes > 59 {
        return Err(Error::InvalidValue(format!(
            "not a UTC offset: {}",
            &text[position..]
        )));
    }
    let total = hours * 60 + minutes;
    Ok((
        &text[..position],
        Some(if bytes[position] == b'-' {
            -total
        } else {
            total
        }),
    ))
}

fn check_date(fields: DateFields) -> Result<()> {
    if fields.year == 0 || fields.year > 9999 {
        return Err(Error::InvalidValue(format!(
            "a year is 0001 to 9999: {:04}",
            fields.year
        )));
    }
    if !(1..=12).contains(&fields.month) {
        return Err(Error::InvalidValue(format!(
            "a month is 01 to 12: {:02}",
            fields.month
        )));
    }
    if fields.day == 0 || fields.day > days_in_month(fields.year, fields.month) {
        return Err(Error::InvalidValue(format!(
            "no day {day} in {year:04}-{month:02}",
            day = fields.day,
            year = fields.year,
            month = fields.month
        )));
    }
    Ok(())
}

fn check_time(fields: TimeFields) -> Result<()> {
    if fields.hour > 23 {
        return Err(Error::InvalidValue(format!(
            "an hour is 00 to 23: {:02}",
            fields.hour
        )));
    }
    if fields.minute > 59 {
        return Err(Error::InvalidValue(format!(
            "a minute is 00 to 59: {:02}",
            fields.minute
        )));
    }
    // A second of 60 is the leap second PS3.5 permits; pydicom lowers it to 59 because Python
    // cannot hold it, and this keeps it.
    if fields.second > 60 {
        return Err(Error::InvalidValue(format!(
            "a second is 00 to 60: {:02}",
            fields.second
        )));
    }
    Ok(())
}

fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        _ => 28,
    }
}

fn is_leap_year(year: u16) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn is_digits(bytes: &[u8]) -> bool {
    bytes.iter().all(u8::is_ascii_digit)
}

/// Read `length` digits starting at `start`. The caller has already checked they are digits.
fn number(text: &str, start: usize, length: usize) -> u16 {
    text[start..start + length].parse().unwrap_or_default()
}
