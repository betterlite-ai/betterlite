use super::super::DateFlags;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Label {
    Year,
    Month,
    Day,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Field {
    pub(super) value: i64,
    pub(super) label: Option<Label>,
}

// the year, month and day list the source's parser fills as it walks the tokens
#[derive(Debug, Clone, Default)]
pub(super) struct Fields {
    pub(super) items: Vec<Field>,
    pub(super) century: bool,
}

impl Fields {
    pub(super) fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub(super) fn len(&self) -> usize {
        self.items.len()
    }

    pub(super) fn index(&self, label: Label) -> Option<usize> {
        self.items
            .iter()
            .position(|field| field.label == Some(label))
    }

    pub(super) fn has(&self, label: Label) -> bool {
        self.index(label).is_some()
    }

    // a run of digits read as text; more than two digits make it the year
    pub(super) fn push_text(&mut self, text: &str, label: Option<Label>) -> Option<()> {
        if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let value: i64 = text.parse().ok()?;
        let mut label = label;
        if text.len() > 2 && label.is_none() {
            self.century = true;
            label = Some(Label::Year);
        }
        self.claim(label)?;
        self.items.push(Field { value, label });
        Some(())
    }

    // a number; anything above 100 is the year, whatever its label
    pub(super) fn push_value(&mut self, value: i64, label: Option<Label>) -> Option<()> {
        let mut label = label;
        if value > 100 {
            self.century = true;
            if label.is_none() {
                label = Some(Label::Year);
            }
        }
        self.claim(label)?;
        self.items.push(Field { value, label });
        Some(())
    }

    fn claim(&mut self, label: Option<Label>) -> Option<()> {
        match label {
            Some(label) if self.has(label) => None,
            Some(_) | None => Some(()),
        }
    }
}

pub(super) fn resolve(
    fields: &Fields,
    flags: DateFlags,
) -> Option<(Option<i64>, Option<i64>, Option<i64>)> {
    let items = &fields.items;
    if items.is_empty() {
        return Some((None, None, None));
    }
    let labeled = [
        fields.index(Label::Year),
        fields.index(Label::Month),
        fields.index(Label::Day),
    ];
    let known = labeled.iter().flatten().count();
    if known == items.len() || (items.len() == 3 && known == 2) {
        return Some(labeled_resolution(fields, labeled));
    }
    let values: Vec<i64> = items.iter().map(|field| field.value).collect();
    match values.len() {
        1 => single(&values, labeled[1]),
        2 => pair(&values, labeled[1], flags),
        3 => triple(&values, labeled[1], fields, flags),
        _ => None,
    }
}

// every element carries a label, or three elements carry two and one is left to back out
fn labeled_resolution(
    fields: &Fields,
    labeled: [Option<usize>; 3],
) -> (Option<i64>, Option<i64>, Option<i64>) {
    let mut slots = labeled;
    if slots.iter().flatten().count() < fields.len() {
        let used: Vec<usize> = slots.iter().flatten().copied().collect();
        let missing = (0..fields.len()).find(|index| !used.contains(index));
        let free = slots.iter().position(Option::is_none);
        if let (Some(missing), Some(free)) = (missing, free) {
            slots[free] = Some(missing);
        }
    }
    let value = |slot: Option<usize>| {
        slot.and_then(|index| fields.items.get(index))
            .map(|field| field.value)
    };
    (value(slots[0]), value(slots[1]), value(slots[2]))
}

fn single(
    values: &[i64],
    month_index: Option<usize>,
) -> Option<(Option<i64>, Option<i64>, Option<i64>)> {
    let Some(value) = values.first().copied() else {
        return Some((None, None, None));
    };
    if let Some(index) = month_index {
        return Some((None, values.get(index).copied(), None));
    }
    if value > 31 {
        return Some((Some(value), None, None));
    }
    Some((None, None, Some(value)))
}

fn pair(
    values: &[i64],
    month_index: Option<usize>,
    flags: DateFlags,
) -> Option<(Option<i64>, Option<i64>, Option<i64>)> {
    let first = values.first().copied()?;
    let second = values.get(1).copied()?;
    if let Some(index) = month_index {
        let month = values.get(index).copied();
        let other = if index == 0 { second } else { first };
        return Some(if other > 31 {
            (Some(other), month, None)
        } else {
            (None, month, Some(other))
        });
    }
    if first > 31 {
        return Some((Some(first), Some(second), None));
    }
    if second > 31 {
        return Some((Some(second), Some(first), None));
    }
    if flags.dayfirst && second <= 12 {
        return Some((None, Some(second), Some(first)));
    }
    Some((None, Some(first), Some(second)))
}

fn triple(
    values: &[i64],
    month_index: Option<usize>,
    fields: &Fields,
    flags: DateFlags,
) -> Option<(Option<i64>, Option<i64>, Option<i64>)> {
    let second = values.get(1).copied()?;
    let slots = match month_index {
        Some(0) if second > 31 => (1, 0, 2),
        Some(0) => (2, 0, 1),
        Some(1) => month_between(values, flags)?,
        Some(2) if second > 31 => (1, 2, 0),
        Some(2) => (0, 2, 1),
        _ => numeric_shape(values, fields, flags)?,
    };
    Some((
        values.get(slots.0).copied(),
        values.get(slots.1).copied(),
        values.get(slots.2).copied(),
    ))
}

fn month_between(values: &[i64], flags: DateFlags) -> Option<(usize, usize, usize)> {
    let first = values.first().copied()?;
    let third = values.get(2).copied()?;
    Some(if first > 31 || (flags.yearfirst && third <= 31) {
        (0, 1, 2)
    } else {
        (2, 1, 0)
    })
}

fn numeric_shape(
    values: &[i64],
    fields: &Fields,
    flags: DateFlags,
) -> Option<(usize, usize, usize)> {
    let first = values.first().copied()?;
    let second = values.get(1).copied()?;
    let third = values.get(2).copied()?;
    if first > 31
        || fields.index(Label::Year) == Some(0)
        || (flags.yearfirst && second <= 12 && third <= 31)
    {
        return Some(if flags.dayfirst && third <= 12 {
            (0, 2, 1)
        } else {
            (0, 1, 2)
        });
    }
    if first > 12 || (flags.dayfirst && second <= 12) {
        return Some((2, 1, 0));
    }
    Some((2, 0, 1))
}

pub(super) fn convert_year(year: i64, century: bool, current_year: i64) -> i64 {
    if year >= 100 || century {
        return year;
    }
    let base = current_year
        .checked_div(100)
        .unwrap_or(0)
        .saturating_mul(100);
    let converted = year.saturating_add(base);
    if converted >= current_year.saturating_add(50) {
        return converted.saturating_sub(100);
    }
    converted
}

pub(super) fn days_in_month(year: i64, month: i64) -> Option<u32> {
    const DAYS: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let index = usize::try_from(month.checked_sub(1)?).ok()?;
    let days = *DAYS.get(index)?;
    if month == 2 && is_leap(year) {
        return Some(29);
    }
    Some(days)
}

fn is_leap(year: i64) -> bool {
    year.rem_euclid(4) == 0 && (year.rem_euclid(100) != 0 || year.rem_euclid(400) == 0)
}
