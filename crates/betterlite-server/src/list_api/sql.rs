use super::plan::types::{AnyOf, ComparisonOp, FilterValue, IsNull, Predicate, SortKey, Within};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SqlRenderError {
    #[error("SQL placeholder index overflow")]
    PlaceholderOverflow,
}

#[must_use]
pub fn order_by_sql(order: &[SortKey]) -> String {
    order
        .iter()
        .map(|key| {
            format!(
                "\"{}\" {} NULLS LAST",
                key.field,
                if key.descending { "DESC" } else { "ASC" }
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn placeholder(index: usize, value: &FilterValue) -> String {
    match value {
        FilterValue::Timestamp(_) => format!("${index}::timestamptz AT TIME ZONE 'UTC'"),
        FilterValue::Text(_) | FilterValue::Int(_) | FilterValue::Money(_) => format!("${index}"),
    }
}

const fn comparison_operator(operator: ComparisonOp) -> &'static str {
    match operator {
        ComparisonOp::Eq => "=",
        ComparisonOp::Not => "<>",
        ComparisonOp::Gte => ">=",
        ComparisonOp::Lte => "<=",
        ComparisonOp::Gt => ">",
        ComparisonOp::Lt => "<",
        ComparisonOp::Contains => "ILIKE",
    }
}

fn render_compare(
    compare: &super::plan::types::Compare,
    index: usize,
) -> (String, Vec<FilterValue>) {
    if compare.op == ComparisonOp::Contains {
        let value = match &compare.value {
            FilterValue::Text(text) => {
                FilterValue::Text(format!("%{}%", super::problem::escape_like(text)))
            }
            FilterValue::Int(_) | FilterValue::Money(_) | FilterValue::Timestamp(_) => {
                compare.value.clone()
            }
        };
        return (
            format!("\"{}\" ILIKE ${index} ESCAPE '\\'", compare.field),
            vec![value],
        );
    }
    (
        format!(
            "\"{}\" {} {}",
            compare.field,
            comparison_operator(compare.op),
            placeholder(index, &compare.value)
        ),
        vec![compare.value.clone()],
    )
}

fn render_within(
    within: &Within,
    index: usize,
) -> Result<(String, Vec<FilterValue>), SqlRenderError> {
    let mut placeholders = Vec::new();
    for (offset, value) in within.values.iter().enumerate() {
        let position = index
            .checked_add(offset)
            .ok_or(SqlRenderError::PlaceholderOverflow)?;
        placeholders.push(placeholder(position, value));
    }
    Ok((
        format!("\"{}\" IN ({})", within.field, placeholders.join(", ")),
        within.values.clone(),
    ))
}

fn render_any_of(
    clauses: &[super::plan::types::Compare],
    index: usize,
) -> Result<(String, Vec<FilterValue>), SqlRenderError> {
    let mut sql_clauses = Vec::new();
    let mut values = Vec::new();
    for clause in clauses {
        let next = index
            .checked_add(values.len())
            .ok_or(SqlRenderError::PlaceholderOverflow)?;
        let (sql, mut bound) = render_compare(clause, next);
        sql_clauses.push(sql);
        values.append(&mut bound);
    }
    Ok((format!("({})", sql_clauses.join(" OR ")), values))
}

fn render(
    predicate: &Predicate,
    index: usize,
) -> Result<(String, Vec<FilterValue>), SqlRenderError> {
    match predicate {
        Predicate::IsNull(IsNull { field, negated }) => Ok((
            format!("\"{field}\" IS {}NULL", if *negated { "NOT " } else { "" }),
            Vec::new(),
        )),
        Predicate::Within(within) => render_within(within, index),
        Predicate::AnyOf(AnyOf { clauses }) => render_any_of(clauses, index),
        Predicate::Compare(compare) => Ok(render_compare(compare, index)),
    }
}

pub fn where_sql(
    where_: &[Predicate],
    first_index: usize,
) -> Result<(String, Vec<FilterValue>), SqlRenderError> {
    let mut clauses = Vec::new();
    let mut params = Vec::new();
    for predicate in where_ {
        let index = first_index
            .checked_add(params.len())
            .ok_or(SqlRenderError::PlaceholderOverflow)?;
        let (clause, mut values) = render(predicate, index)?;
        clauses.push(clause);
        params.append(&mut values);
    }
    Ok((clauses.join(" AND "), params))
}

mod tests;
