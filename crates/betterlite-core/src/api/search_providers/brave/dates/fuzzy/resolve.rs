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

// the calendar members a run of numbers names; a member the source fills from its own day is None
#[derive(Debug, Clone, Copy)]
pub(super) struct Resolved(
    pub(super) Option<i64>,
    pub(super) Option<i64>,
    pub(super) Option<i64>,
);

// the year, month and day list the source's parser fills as it walks the tokens
#[derive(Debug, Clone, Default)]
pub(super) struct Fields {
    pub(super) items: Vec<Field>,
    pub(super) century: bool,
}

impl Fields {
    pub(super) fn index(&self, label: Label) -> Option<usize> {
        self.items
            .iter()
            .position(|field| field.label == Some(label))
    }

    // a run of digits read as text; more than two digits make it the year
    pub(super) fn push_text(&mut self, text: &str, label: Option<Label>) -> Option<()> {
        if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let long = text.len() > 2;
        self.push(text.parse().ok()?, label, long)
    }

    pub(super) fn push(&mut self, value: i64, label: Option<Label>, long: bool) -> Option<()> {
        self.century |= long;
        let named = label.or_else(|| long.then_some(Label::Year));
        if named.is_some_and(|chosen| self.index(chosen).is_some()) {
            return None;
        }
        self.items.push(Field {
            value,
            label: named,
        });
        Some(())
    }
}

pub(super) fn resolve(fields: &Fields, flags: DateFlags) -> Option<Resolved> {
    let items = &fields.items;
    if items.is_empty() {
        return Some(Resolved(None, None, None));
    }
    let mut slots = [
        fields.index(Label::Year),
        fields.index(Label::Month),
        fields.index(Label::Day),
    ];
    let known = slots.iter().flatten().count();
    if known == items.len() || (items.len() == 3 && known == 2) {
        let missing =
            (0..items.len()).find(|index| slots.iter().flatten().all(|slot| slot != index));
        if let (Some(unlabeled), Some(free)) = (missing, slots.iter().position(Option::is_none)) {
            *slots.get_mut(free)? = Some(unlabeled);
        }
        let value = |slot: Option<usize>| {
            slot.and_then(|index| items.get(index))
                .map(|field| field.value)
        };
        let [year, month, day] = slots;
        return Some(Resolved(value(year), value(month), value(day)));
    }
    let month_index = slots.get(1).copied().flatten();
    let values: Vec<i64> = items.iter().map(|field| field.value).collect();
    match values.len() {
        1 => single(&values, month_index),
        2 => pair(&values, month_index, flags),
        3 => triple(&values, month_index, fields, flags),
        _ => None,
    }
}

fn single(values: &[i64], month_index: Option<usize>) -> Option<Resolved> {
    let value = values.first().copied()?;
    if month_index.is_some() {
        return Some(Resolved(None, Some(value), None));
    }
    if value > 31 {
        return Some(Resolved(Some(value), None, None));
    }
    Some(Resolved(None, None, Some(value)))
}

fn pair(values: &[i64], month_index: Option<usize>, flags: DateFlags) -> Option<Resolved> {
    let first = values.first().copied()?;
    let second = values.get(1).copied()?;
    if let Some(index) = month_index {
        let other = if index == 0 { second } else { first };
        let month = values.get(index).copied();
        return Some(if other > 31 {
            Resolved(Some(other), month, None)
        } else {
            Resolved(None, month, Some(other))
        });
    }
    if first > 31 {
        return Some(Resolved(Some(first), Some(second), None));
    }
    if second > 31 {
        return Some(Resolved(Some(second), Some(first), None));
    }
    if flags.dayfirst && second <= 12 {
        return Some(Resolved(None, Some(second), Some(first)));
    }
    Some(Resolved(None, Some(first), Some(second)))
}

// where each member of a three-number run lands, as year, month and day positions
fn triple(
    values: &[i64],
    month_index: Option<usize>,
    fields: &Fields,
    flags: DateFlags,
) -> Option<Resolved> {
    let second = values.get(1).copied()?;
    let slots = match month_index {
        Some(0) if second > 31 => [1, 0, 2],
        Some(0) => [2, 0, 1],
        Some(1) => {
            let first = values.first().copied()?;
            let third = values.get(2).copied()?;
            if first > 31 || (flags.yearfirst && third <= 31) {
                [0, 1, 2]
            } else {
                [2, 1, 0]
            }
        }
        Some(2) if second > 31 => [1, 2, 0],
        Some(2) => [0, 2, 1],
        _ => numeric_shape(values, fields, flags)?,
    };
    let [year, month, day] = slots;
    Some(Resolved(
        values.get(year).copied(),
        values.get(month).copied(),
        values.get(day).copied(),
    ))
}

fn numeric_shape(values: &[i64], fields: &Fields, flags: DateFlags) -> Option<[usize; 3]> {
    let first = values.first().copied()?;
    let second = values.get(1).copied()?;
    let third = values.get(2).copied()?;
    if first > 31
        || fields.index(Label::Year) == Some(0)
        || (flags.yearfirst && second <= 12 && third <= 31)
    {
        return Some(if flags.dayfirst && third <= 12 {
            [0, 2, 1]
        } else {
            [0, 1, 2]
        });
    }
    if first > 12 || (flags.dayfirst && second <= 12) {
        return Some([2, 1, 0]);
    }
    Some([2, 0, 1])
}

pub(super) const fn convert_year(year: i64, century: bool, current_year: i64) -> i64 {
    if year >= 100 || century {
        return year;
    }
    let base = current_year.div_euclid(100).saturating_mul(100);
    let converted = year.saturating_add(base);
    if converted >= current_year.saturating_add(50) {
        converted.saturating_sub(100)
    } else {
        converted
    }
}

pub(super) fn days_in_month(year: i64, month: i64) -> Option<u32> {
    const DAYS: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let days = *DAYS.get(usize::try_from(month.checked_sub(1)?).ok()?)?;
    let leap = year.rem_euclid(4) == 0 && (year.rem_euclid(100) != 0 || year.rem_euclid(400) == 0);
    Some(if month == 2 && leap { 29 } else { days })
}
