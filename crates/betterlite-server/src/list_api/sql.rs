use super::plan::types::{AnyOf, ComparisonOp, FilterValue, IsNull, Predicate, SortKey, Within};

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

fn value_sql(value: &FilterValue) -> String {
    match value {
        FilterValue::Text(text) => format!("%{}%", super::problem::escape_like(text)),
        FilterValue::Int(number) => number.to_string(),
        FilterValue::Money(amount) => amount.to_string(),
        FilterValue::Timestamp(timestamp) => timestamp.to_rfc3339(),
    }
}

fn render(predicate: &Predicate, index: usize) -> (String, Vec<String>) {
    match predicate {
        Predicate::IsNull(IsNull { field, negated }) => (
            format!("\"{field}\" IS {}NULL", if *negated { "NOT " } else { "" }),
            Vec::new(),
        ),
        Predicate::Within(Within { field, values }) => {
            let placeholders = values
                .iter()
                .enumerate()
                .map(|(offset, value)| placeholder(index + offset, value))
                .collect::<Vec<_>>();
            (
                format!("\"{field}\" IN ({})", placeholders.join(", ")),
                values.iter().map(value_sql).collect(),
            )
        }
        Predicate::AnyOf(AnyOf { clauses }) => {
            let mut next = index;
            let mut clauses_sql = Vec::new();
            let mut values = Vec::new();
            for clause in clauses {
                let (sql, mut bound) = render(&Predicate::Compare(clause.clone()), next);
                next += bound.len();
                clauses_sql.push(sql);
                values.append(&mut bound);
            }
            (format!("({})", clauses_sql.join(" OR ")), values)
        }
        Predicate::Compare(compare) => {
            let operator = match compare.op {
                ComparisonOp::Eq => "=",
                ComparisonOp::Not => "<>",
                ComparisonOp::Gte => ">=",
                ComparisonOp::Lte => "<=",
                ComparisonOp::Gt => ">",
                ComparisonOp::Lt => "<",
                ComparisonOp::Contains => "ILIKE",
            };
            let value = placeholder(index, &compare.value);
            let sql = if compare.op == ComparisonOp::Contains {
                format!("\"{}\" ILIKE ${index} ESCAPE '\\\\'", compare.field)
            } else {
                format!("\"{}\" {operator} {value}", compare.field)
            };
            (sql, vec![value_sql(&compare.value)])
        }
    }
}

#[must_use]
pub fn where_sql(where_: &[Predicate], first_index: usize) -> (String, Vec<String>) {
    let mut clauses = Vec::new();
    let mut params = Vec::new();
    for predicate in where_ {
        let (clause, mut values) = render(predicate, first_index + params.len());
        clauses.push(clause);
        params.append(&mut values);
    }
    (clauses.join(" AND "), params)
}
