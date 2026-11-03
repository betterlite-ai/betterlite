use super::plan::types::{ComparisonOp, FilterValue, Predicate, QueryPlan, SortKey};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use std::cmp::Ordering;
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cell {
    Text(String),
    Int(i64),
    Money(Decimal),
    Timestamp(DateTime<Utc>),
    Null,
    Repeated(Vec<Cell>),
}
pub type Cells = std::collections::BTreeMap<String, Cell>;

fn sign(cell: &Cell, value: &FilterValue) -> Option<Ordering> {
    match (cell, value) {
        (Cell::Text(left), FilterValue::Text(right)) => Some(left.cmp(right)),
        (Cell::Int(left), FilterValue::Int(right)) => Some(left.cmp(right)),
        (Cell::Money(left), FilterValue::Money(right)) => Some(left.cmp(right)),
        (Cell::Timestamp(left), FilterValue::Timestamp(right)) => Some(left.cmp(right)),
        _ => None,
    }
}

fn holds_one(cell: &Cell, predicate: &Predicate) -> bool {
    match predicate {
        Predicate::Compare(compare) => {
            let ordering = sign(cell, &compare.value);
            match compare.op {
                ComparisonOp::Eq => ordering == Some(Ordering::Equal),
                ComparisonOp::Not => ordering.is_some_and(|value| value != Ordering::Equal),
                ComparisonOp::Gt => ordering == Some(Ordering::Greater),
                ComparisonOp::Gte => matches!(ordering, Some(Ordering::Greater | Ordering::Equal)),
                ComparisonOp::Lt => ordering == Some(Ordering::Less),
                ComparisonOp::Lte => matches!(ordering, Some(Ordering::Less | Ordering::Equal)),
                ComparisonOp::Contains => match (cell, &compare.value) {
                    (Cell::Text(left), FilterValue::Text(right)) => {
                        left.to_lowercase().contains(&right.to_lowercase())
                    }
                    _ => false,
                },
            }
        }
        Predicate::Within(within) => within
            .values
            .iter()
            .any(|value| sign(cell, value) == Some(Ordering::Equal)),
        Predicate::IsNull(is_null) => (matches!(cell, Cell::Null)) != is_null.negated,
        Predicate::AnyOf(any) => any
            .clauses
            .iter()
            .any(|clause| holds_one(cell, &Predicate::Compare(clause.clone()))),
    }
}

fn holds(cell: Option<&Cell>, predicate: &Predicate) -> bool {
    match cell {
        Some(Cell::Repeated(values)) => values.iter().any(|value| holds_one(value, predicate)),
        Some(value) => holds_one(value, predicate),
        None => matches!(predicate, Predicate::IsNull(is_null) if !is_null.negated),
    }
}

fn compare_cells(left: Option<&Cell>, right: Option<&Cell>, key: &SortKey) -> Ordering {
    let ordering = match (left, right) {
        (None | Some(Cell::Null), None | Some(Cell::Null)) => Ordering::Equal,
        (None | Some(Cell::Null), _) => Ordering::Greater,
        (_, None | Some(Cell::Null)) => Ordering::Less,
        (Some(Cell::Repeated(left)), Some(Cell::Repeated(right))) => left.len().cmp(&right.len()),
        (Some(left), Some(right)) => sign(left, &cell_value(right)).unwrap_or(Ordering::Equal),
    };
    if key.descending
        && left.is_some_and(|cell| !matches!(cell, Cell::Null))
        && right.is_some_and(|cell| !matches!(cell, Cell::Null))
    {
        ordering.reverse()
    } else {
        ordering
    }
}

fn cell_value(cell: &Cell) -> FilterValue {
    match cell {
        Cell::Text(value) => FilterValue::Text(value.clone()),
        Cell::Int(value) => FilterValue::Int(*value),
        Cell::Money(value) => FilterValue::Money(*value),
        Cell::Timestamp(value) => FilterValue::Timestamp(*value),
        Cell::Null | Cell::Repeated(_) => FilterValue::Text(String::new()),
    }
}

#[derive(Debug, Clone)]
pub struct InMemoryListExecutor<Row> {
    pub rows: Vec<(Row, Cells)>,
}

impl<Row: Clone> InMemoryListExecutor<Row> {
    pub fn count(&self, where_: &[Predicate]) -> u64 {
        u64::try_from(
            self.rows
                .iter()
                .filter(|(_, cells)| {
                    where_
                        .iter()
                        .all(|predicate| holds(cells.get(&predicate_field(predicate)), predicate))
                })
                .count(),
        )
        .unwrap_or(u64::MAX)
    }

    pub fn find_many(&self, plan: &QueryPlan) -> Vec<Row> {
        let mut rows: Vec<&(Row, Cells)> = self
            .rows
            .iter()
            .filter(|(_, cells)| {
                plan.where_
                    .iter()
                    .all(|predicate| holds(cells.get(&predicate_field(predicate)), predicate))
            })
            .collect();
        for key in plan.order.iter().rev() {
            rows.sort_by(|left, right| {
                compare_cells(left.1.get(&key.field), right.1.get(&key.field), key)
            });
            if key.descending {
                rows.reverse();
            }
        }
        let skip = usize::try_from(plan.skip).unwrap_or(usize::MAX);
        let take = usize::try_from(plan.take).unwrap_or(usize::MAX);
        rows.into_iter()
            .skip(skip)
            .take(take)
            .map(|(row, _)| row.clone())
            .collect()
    }

    pub fn distinct(&self, field: &str, where_: &[Predicate]) -> Vec<String> {
        let mut values = BTreeSet::new();
        for (_, cells) in self.rows.iter().filter(|(_, cells)| {
            where_
                .iter()
                .all(|predicate| holds(cells.get(&predicate_field(predicate)), predicate))
        }) {
            match cells.get(field) {
                Some(Cell::Text(value)) if !value.is_empty() => {
                    values.insert(value.clone());
                }
                Some(Cell::Repeated(items)) => {
                    for item in items {
                        if let Cell::Text(value) = item {
                            if !value.is_empty() {
                                values.insert(value.clone());
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        values.into_iter().collect()
    }
}

fn predicate_field(predicate: &Predicate) -> String {
    match predicate {
        Predicate::Compare(value) => value.field.clone(),
        Predicate::Within(value) => value.field.clone(),
        Predicate::IsNull(value) => value.field.clone(),
        Predicate::AnyOf(value) => value
            .clauses
            .first()
            .map(|clause| clause.field.clone())
            .unwrap_or_default(),
    }
}
