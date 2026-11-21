use chrono::{Datelike as _, Days, NaiveDate};

use resolve::{Fields, Resolved};

use super::DateFlags;

mod parser;
mod resolve;
mod tokens;

pub(super) fn parse_date(text: &str, today: NaiveDate, flags: DateFlags) -> Option<NaiveDate> {
    let tokens = tokens::tokenize(text);
    let mut parser = parser::Parser {
        tokens: &tokens,
        today,
        flags,
        fields: Fields::default(),
        weekday: None,
        saw: false,
    };
    parser.run()?;
    build(&parser)
}

// the source fills an incomplete date from the day it parses on, then reads its weekday
fn build(parser: &parser::Parser<'_>) -> Option<NaiveDate> {
    if !parser.saw && parser.fields.items.is_empty() {
        return None;
    }
    let Resolved(year, month, day) = resolve::resolve(&parser.fields, parser.flags)?;
    let has_day = day.is_some();
    let current = i64::from(parser.today.year());
    let resolved_year = year.map_or(current, |parsed| {
        resolve::convert_year(parsed, parser.fields.century, current)
    });
    if !(1..=9999).contains(&resolved_year) {
        return None;
    }
    let selected_month = month.unwrap_or_else(|| i64::from(parser.today.month()));
    let last = i64::from(resolve::days_in_month(resolved_year, selected_month)?);
    let selected_day = day.unwrap_or_else(|| i64::from(parser.today.day()).min(last));
    let date = NaiveDate::from_ymd_opt(
        i32::try_from(resolved_year).ok()?,
        u32::try_from(selected_month).ok()?,
        u32::try_from(selected_day).ok()?,
    )?;
    match parser.weekday {
        Some(weekday) if !has_day => shift_to_weekday(date, weekday),
        Some(_) | None => Some(date),
    }
}

// a weekday without a day names that weekday in the week of the date, today included
fn shift_to_weekday(date: NaiveDate, weekday: i64) -> Option<NaiveDate> {
    let current = i64::from(date.weekday().num_days_from_monday());
    let days = u64::try_from(weekday.checked_sub(current)?.rem_euclid(7)).ok()?;
    date.checked_add_days(Days::new(days))
}
