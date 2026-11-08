use super::SchedulerError;

const WEEKDAY_NAMES: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
const MONTH_NAMES: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CronField {
    Minute,
    Hour,
    DayOfMonth,
    Month,
    DayOfWeek,
}

impl CronField {
    const fn bounds(self) -> (u64, u64) {
        match self {
            Self::Minute => (0, 59),
            Self::Hour => (0, 23),
            Self::DayOfMonth => (1, 31),
            Self::Month => (1, 12),
            Self::DayOfWeek => (0, 6),
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Minute => "minute",
            Self::Hour => "hour",
            Self::DayOfMonth => "day",
            Self::Month => "month",
            Self::DayOfWeek => "day_of_week",
        }
    }

    const fn names(self) -> Option<&'static [&'static str]> {
        match self {
            Self::Month => Some(&MONTH_NAMES),
            Self::DayOfWeek => Some(&WEEKDAY_NAMES),
            Self::Minute | Self::Hour | Self::DayOfMonth => None,
        }
    }
}

pub(crate) fn normalize_field(field: CronField, text: &str) -> Result<String, SchedulerError> {
    let terms = text
        .split(',')
        .map(|term| normalize_term(field, term))
        .collect::<Result<Vec<_>, _>>()?;
    // A star matches every value whatever the other terms say, and croner rejects one inside a list.
    if terms.iter().any(|term| term.as_str() == "*") {
        return Ok("*".to_owned());
    }
    Ok(terms.join(","))
}

// A name expression matches its prefix, so whatever follows the second name is ignored as APScheduler does.
fn normalize_term(field: CronField, term: &str) -> Result<String, SchedulerError> {
    let lowered = term.to_ascii_lowercase();
    if field == CronField::DayOfMonth && lowered.starts_with("last") {
        return Ok("L".to_owned());
    }
    if let Some(names) = field.names()
        && lowered.starts_with(|first: char| first.is_ascii_alphabetic())
    {
        return normalize_name_term(field, names, &lowered);
    }
    normalize_numeric_term(field, term)
}

fn normalize_name_term(
    field: CronField,
    names: &[&str],
    lowered: &str,
) -> Result<String, SchedulerError> {
    let (first_text, rest) = split_name(lowered);
    let first = names
        .iter()
        .position(|candidate| *candidate == first_text)
        .ok_or_else(|| invalid_name(field, first_text))?;
    let Some(last_text) = rest else {
        return Ok(first_text.to_owned());
    };
    let last = names
        .iter()
        .position(|candidate| *candidate == last_text)
        .ok_or_else(|| invalid_name(field, last_text))?;
    if first > last {
        return Err(SchedulerError::CronRangeOrder);
    }
    if first == last {
        return Ok(first_text.to_owned());
    }
    Ok(format!("{first_text}-{last_text}"))
}

fn split_name(lowered: &str) -> (&str, Option<&str>) {
    let first_end = lowered
        .find(|value: char| !value.is_ascii_alphabetic())
        .unwrap_or(lowered.len());
    let Some((first, rest)) = lowered.split_at_checked(first_end) else {
        return (lowered, None);
    };
    let Some(after_dash) = rest.strip_prefix('-') else {
        return (first, None);
    };
    let last_end = after_dash
        .find(|value: char| !value.is_ascii_alphabetic())
        .unwrap_or(after_dash.len());
    let Some((last, _tail)) = after_dash.split_at_checked(last_end) else {
        return (first, None);
    };
    if last.is_empty() {
        (first, None)
    } else {
        (first, Some(last))
    }
}

fn normalize_numeric_term(field: CronField, term: &str) -> Result<String, SchedulerError> {
    let (base, step) = match term.split_once('/') {
        Some((base, text)) => (base, Some(parse_count(field, term, text)?)),
        None => (term, None),
    };
    if base == "*" {
        if step == Some(0) {
            return Err(SchedulerError::CronStepZero);
        }
        let (minimum, maximum) = field.bounds();
        check_step_against(term, step, maximum.saturating_sub(minimum))?;
        return match (field, step) {
            (CronField::DayOfWeek, None) => Ok("*".to_owned()),
            (CronField::DayOfWeek, Some(width)) => weekday_list(0, 6, width, term),
            _ => Ok(term.to_owned()),
        };
    }
    let (first, last) = parse_range(field, term, base)?;
    check_numeric(field, term, first, last, step)?;
    if field != CronField::DayOfWeek {
        return Ok(term.to_owned());
    }
    match (last, step) {
        (Some(bound), _) if bound != first => Ok(format!(
            "{}-{}{}",
            weekday_name(first, term)?,
            weekday_name(bound, term)?,
            step.map_or_else(String::new, |width| format!("/{width}"))
        )),
        (None, Some(width)) => weekday_list(first, 6, width, term),
        _ => Ok(weekday_name(first, term)?.to_owned()),
    }
}

fn parse_range(
    field: CronField,
    expression: &str,
    base: &str,
) -> Result<(u64, Option<u64>), SchedulerError> {
    match base.split_once('-') {
        Some((first_text, last_text)) => Ok((
            parse_count(field, expression, first_text)?,
            Some(parse_count(field, expression, last_text)?),
        )),
        None => Ok((parse_count(field, expression, base)?, None)),
    }
}

fn parse_count(field: CronField, expression: &str, text: &str) -> Result<u64, SchedulerError> {
    // APScheduler's value regex is \d+, so a sign or any other byte makes the expression unrecognized.
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid_expression(field, expression));
    }
    text.parse::<u64>()
        .map_err(|_source| invalid_expression(field, expression))
}

// The check order is APScheduler's: a zero step, a descending range, then the field bounds and step.
fn check_numeric(
    field: CronField,
    expression: &str,
    first: u64,
    last: Option<u64>,
    step: Option<u64>,
) -> Result<(), SchedulerError> {
    if step == Some(0) {
        return Err(SchedulerError::CronStepZero);
    }
    if let Some(bound) = last
        && first > bound
    {
        return Err(SchedulerError::CronRangeOrder);
    }
    let (minimum, maximum) = field.bounds();
    check_step_against(expression, step, maximum.saturating_sub(minimum))?;
    if first < minimum {
        return Err(SchedulerError::CronBelowMin {
            expression: expression.to_owned(),
            value: first,
            minimum,
        });
    }
    let bounded = bounded_last(first, last, step);
    if let Some(bound) = bounded
        && bound > maximum
    {
        return Err(SchedulerError::CronAboveMax {
            expression: expression.to_owned(),
            value: bound,
            maximum,
        });
    }
    let end = bounded.unwrap_or(maximum);
    check_step_against(expression, step, end.saturating_sub(first))
}

fn check_step_against(
    expression: &str,
    step: Option<u64>,
    range: u64,
) -> Result<(), SchedulerError> {
    let Some(width) = step else {
        return Ok(());
    };
    if width > range {
        return Err(SchedulerError::CronStep {
            expression: expression.to_owned(),
            step: width,
            range,
        });
    }
    Ok(())
}

// A bare value gets last = first and a value with a step keeps no upper bound, as APScheduler does.
const fn bounded_last(first: u64, last: Option<u64>, step: Option<u64>) -> Option<u64> {
    match (last, step) {
        (Some(bound), _) => Some(bound),
        (None, None) => Some(first),
        (None, Some(_width)) => None,
    }
}

fn weekday_list(
    start: u64,
    end: u64,
    step: u64,
    expression: &str,
) -> Result<String, SchedulerError> {
    let mut names = Vec::new();
    let mut value = start;
    // The star branch and check_numeric reject a zero step before this walk.
    while value <= end {
        names.push(weekday_name(value, expression)?);
        value = value.saturating_add(step);
    }
    Ok(names.join(","))
}

fn weekday_name(value: u64, expression: &str) -> Result<&'static str, SchedulerError> {
    let index = usize::try_from(value)
        .map_err(|_source| invalid_expression(CronField::DayOfWeek, expression))?;
    WEEKDAY_NAMES
        .get(index)
        .copied()
        .ok_or_else(|| invalid_expression(CronField::DayOfWeek, expression))
}

fn invalid_name(field: CronField, name: &str) -> SchedulerError {
    let label = if field == CronField::Month {
        "month"
    } else {
        "weekday"
    };
    SchedulerError::CronName {
        field: label,
        name: name.to_owned(),
    }
}

fn invalid_expression(field: CronField, expression: &str) -> SchedulerError {
    SchedulerError::CronExpression {
        field: field.label(),
        expression: expression.to_owned(),
    }
}
