use super::plan::types::{ComparisonOp, FilterValue, Predicate, QueryPlan, SortKey};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::pin::Pin;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cell {
    Text(String),
    Int(i64),
    Money(Decimal),
    Timestamp(DateTime<Utc>),
    Null,
    Repeated(Vec<Self>),
}
pub type Cells = BTreeMap<String, Cell>;

impl Cell {
    fn compare(&self, other: &Self) -> Option<Ordering> {
        match (self, other) {
            (Self::Text(left), Self::Text(right)) => Some(left.cmp(right)),
            (Self::Int(left), Self::Int(right)) => Some(left.cmp(right)),
            (Self::Money(left), Self::Money(right)) => Some(left.cmp(right)),
            (Self::Int(left), Self::Money(right)) => Some(Decimal::from(*left).cmp(right)),
            (Self::Money(left), Self::Int(right)) => Some(left.cmp(&Decimal::from(*right))),
            (Self::Timestamp(left), Self::Timestamp(right)) => Some(left.cmp(right)),
            _ => None,
        }
    }
}

fn equal(left_cell: &Cell, right_cell: &Cell) -> bool {
    match (left_cell, right_cell) {
        (Cell::Text(lhs), Cell::Text(rhs)) => lhs == rhs,
        (Cell::Int(lhs), Cell::Int(rhs)) => lhs == rhs,
        (Cell::Money(lhs), Cell::Money(rhs)) => lhs == rhs,
        (Cell::Int(lhs), Cell::Money(rhs)) => Decimal::from(*lhs) == *rhs,
        (Cell::Money(lhs), Cell::Int(rhs)) => *lhs == Decimal::from(*rhs),
        (Cell::Timestamp(lhs), Cell::Timestamp(rhs)) => lhs == rhs,
        _ => false,
    }
}

fn holds_one(cell: &Cell, predicate: &Predicate) -> bool {
    match predicate {
        Predicate::Compare(compare) => {
            let ordering = cell.compare(&filter_cell(&compare.value));
            match compare.op {
                ComparisonOp::Eq => equal(cell, &filter_cell(&compare.value)),
                ComparisonOp::Not => {
                    !matches!(cell, Cell::Null) && !equal(cell, &filter_cell(&compare.value))
                }
                ComparisonOp::Gt => ordering == Some(Ordering::Greater),
                ComparisonOp::Gte => matches!(ordering, Some(Ordering::Greater | Ordering::Equal)),
                ComparisonOp::Lt => ordering == Some(Ordering::Less),
                ComparisonOp::Lte => matches!(ordering, Some(Ordering::Less | Ordering::Equal)),
                ComparisonOp::Contains => match (cell, &compare.value) {
                    (Cell::Text(left), FilterValue::Text(right)) => {
                        caseless::default_case_fold_str(left)
                            .contains(&caseless::default_case_fold_str(right))
                    }
                    _ => false,
                },
            }
        }
        Predicate::Within(within) => within
            .values
            .iter()
            .any(|value| cell.compare(&filter_cell(value)) == Some(Ordering::Equal)),
        Predicate::IsNull(is_null) => (matches!(cell, Cell::Null)) != is_null.negated,
        Predicate::AnyOf(any) => any
            .clauses
            .iter()
            .any(|clause| holds_one(cell, &Predicate::Compare(clause.clone()))),
    }
}

fn filter_cell(value: &FilterValue) -> Cell {
    match value {
        FilterValue::Text(text) => Cell::Text(text.clone()),
        FilterValue::Int(number) => Cell::Int(*number),
        FilterValue::Money(amount) => Cell::Money(*amount),
        FilterValue::Timestamp(timestamp) => Cell::Timestamp(*timestamp),
    }
}

fn holds(cells: &Cells, predicate: &Predicate) -> bool {
    if let Predicate::AnyOf(any) = predicate {
        return any.clauses.iter().any(|clause| {
            holds_field(
                cells.get(&clause.field),
                &Predicate::Compare(clause.clone()),
            )
        });
    }
    holds_field(cells.get(predicate_field(predicate)), predicate)
}

fn holds_field(cell: Option<&Cell>, predicate: &Predicate) -> bool {
    match cell {
        Some(Cell::Repeated(values)) => values.iter().any(|value| holds_one(value, predicate)),
        Some(value) => holds_one(value, predicate),
        None => matches!(predicate, Predicate::IsNull(is_null) if !is_null.negated),
    }
}

pub type EnricherFuture<Row> = Pin<Box<dyn Future<Output = Vec<Row>> + Send>>;
type PageEnricher<Row> = Arc<dyn Fn(Vec<Row>) -> EnricherFuture<Row> + Send + Sync>;

fn compare_cells(left_cell: Option<&Cell>, right_cell: Option<&Cell>, key: &SortKey) -> Ordering {
    let ordering = match (left_cell, right_cell) {
        (None | Some(Cell::Null), None | Some(Cell::Null)) => Ordering::Equal,
        (None | Some(Cell::Null), _) => Ordering::Greater,
        (_, None | Some(Cell::Null)) => Ordering::Less,
        (Some(Cell::Repeated(left_values)), Some(Cell::Repeated(right_values))) => {
            compare_repeated(left_values, right_values)
        }
        (Some(left_value), Some(right_value)) => {
            left_value.compare(right_value).unwrap_or(Ordering::Equal)
        }
    };
    if key.descending
        && left_cell.is_some_and(|cell| !matches!(cell, Cell::Null))
        && right_cell.is_some_and(|cell| !matches!(cell, Cell::Null))
    {
        ordering.reverse()
    } else {
        ordering
    }
}

fn compare_repeated(left: &[Cell], right: &[Cell]) -> Ordering {
    for (left_value, right_value) in left.iter().zip(right) {
        let ordering = left_value.compare(right_value).unwrap_or(Ordering::Equal);
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    left.len().cmp(&right.len())
}

#[derive(Clone)]
pub struct InMemoryListExecutor<Row> {
    rows: Vec<(Row, Cells)>,
    enrich_page: PageEnricher<Row>,
}

impl<Row: Clone + Send + 'static> InMemoryListExecutor<Row> {
    #[must_use]
    pub fn new(rows: Vec<(Row, Cells)>) -> Self {
        Self {
            rows,
            enrich_page: Arc::new(|page| Box::pin(async move { page })),
        }
    }

    #[must_use]
    pub fn with_enricher<Enricher>(rows: Vec<(Row, Cells)>, enrich_page: Enricher) -> Self
    where
        Enricher: Fn(Vec<Row>) -> EnricherFuture<Row> + Send + Sync + 'static,
    {
        Self {
            rows,
            enrich_page: Arc::new(enrich_page),
        }
    }

    #[must_use]
    pub fn count_rows(&self, where_: &[Predicate]) -> u64 {
        u64::try_from(
            self.rows
                .iter()
                .filter(|(_, cells)| where_.iter().all(|predicate| holds(cells, predicate)))
                .count(),
        )
        .unwrap_or(u64::MAX)
    }

    #[must_use]
    pub async fn find_page<Output, Enrich, EnrichFuture>(
        &self,
        plan: &QueryPlan,
        enrich_page: Enrich,
    ) -> Output
    where
        Enrich: FnOnce(Vec<Row>) -> EnrichFuture + Send,
        EnrichFuture: Future<Output = Output> + Send,
        Output: Send,
        Row: Sync,
    {
        let mut rows: Vec<&(Row, Cells)> = self
            .rows
            .iter()
            .filter(|(_, cells)| plan.where_.iter().all(|predicate| holds(cells, predicate)))
            .collect();
        for key in plan.order.iter().rev() {
            rows.sort_by(|left, right| {
                compare_cells(left.1.get(&key.field), right.1.get(&key.field), key)
            });
        }
        let skip = usize::try_from(plan.skip).unwrap_or(usize::MAX);
        let take = usize::try_from(plan.take).unwrap_or(usize::MAX);
        let page = rows
            .into_iter()
            .skip(skip)
            .take(take)
            .map(|(row, _)| row.clone())
            .collect();
        enrich_page(page).await
    }

    #[must_use]
    pub fn distinct_values(&self, field: &str, where_: &[Predicate]) -> Vec<String> {
        let mut values = BTreeSet::new();
        for (_, cells) in self
            .rows
            .iter()
            .filter(|(_, cells)| where_.iter().all(|predicate| holds(cells, predicate)))
        {
            match cells.get(field) {
                Some(Cell::Text(value)) if !value.is_empty() => {
                    values.insert(value.clone());
                }
                Some(Cell::Repeated(items)) => add_repeated_facet_values(&mut values, items),
                _ => {}
            }
        }
        values.into_iter().collect()
    }
}

impl<Row: Clone + Send + Sync + 'static> super::plan::types::ListExecutor<Row>
    for InMemoryListExecutor<Row>
{
    fn count(&self, where_: &[Predicate]) -> impl Future<Output = u64> + Send {
        ready(self.count_rows(where_))
    }

    fn find_many(&self, plan: &QueryPlan) -> impl Future<Output = Vec<Row>> + Send {
        let page = self.find_page(plan, ready);
        async move { (self.enrich_page)(page.await).await }
    }
}

impl<Row: Clone + Send + Sync + 'static> super::plan::types::FacetExecutor
    for InMemoryListExecutor<Row>
{
    fn distinct(
        &self,
        field: &str,
        where_: &[Predicate],
    ) -> impl Future<Output = Vec<String>> + Send {
        ready(self.distinct_values(field, where_))
    }
}

mod tests;

fn add_facet_value(values: &mut BTreeSet<String>, item: &Cell) {
    if let Cell::Text(value) = item
        && !value.is_empty()
    {
        values.insert(value.clone());
    }
}

fn add_repeated_facet_values(values: &mut BTreeSet<String>, items: &[Cell]) {
    for item in items {
        add_facet_value(values, item);
    }
}

fn predicate_field(predicate: &Predicate) -> &str {
    match predicate {
        Predicate::Compare(value) => &value.field,
        Predicate::Within(value) => &value.field,
        Predicate::IsNull(value) => &value.field,
        Predicate::AnyOf(_) => "",
    }
}
