use std::collections::BTreeMap;
use std::future::Future;

use betterlite_server::list_api::memory::{Cells, InMemoryListExecutor};
use betterlite_server::list_api::plan::types::{
    FacetExecutor as _, FilterValue, ListExecutor as _, QueryPlan, SortKey,
};
use betterlite_server::list_api::sql::{order_by_sql, where_sql};
use serde_json::{Value, json};

use super::parse::{Case, Input, PredicateInput, cell};

pub(super) fn run(input: &Input) -> Result<Value, Box<crate::helpers::HelperError>> {
    let mut output = serde_json::Map::new();
    for (name, case) in &input.cases {
        output.insert(name.clone(), evaluate(case)?);
    }
    Ok(Value::Object(output))
}

fn evaluate(case: &Case) -> Result<Value, Box<crate::helpers::HelperError>> {
    match case {
        Case::Sql {
            predicates: inputs,
            first_index,
            order,
        } => evaluate_sql(inputs.clone(), *first_index, order),
        Case::Memory {
            predicates: inputs,
            rows,
            order,
            distinct_field,
        } => evaluate_memory(inputs.clone(), rows, order, distinct_field.as_deref()),
    }
}

fn evaluate_sql(
    inputs: Vec<PredicateInput>,
    first_index: usize,
    order: &[SortKey],
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let where_ = inputs
        .into_iter()
        .map(super::parse::predicate)
        .collect::<Result<Vec<_>, _>>()?;
    let (sql, params) = where_sql(&where_, first_index)
        .map_err(|error| helper_error!("SqlRenderError", error.to_string()))?;
    let bound_params = params.iter().map(filter_json).collect::<Vec<_>>();
    Ok(json!({"where": sql, "params": bound_params, "order": order_by_sql(order)}))
}

fn evaluate_memory(
    inputs: Vec<PredicateInput>,
    rows: &[BTreeMap<String, super::parse::CellInput>],
    order: &[SortKey],
    distinct_field: Option<&str>,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let where_ = inputs
        .into_iter()
        .map(super::parse::predicate)
        .collect::<Result<Vec<_>, _>>()?;
    let row_cells = rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|(name, value)| (name.clone(), cell(value)))
                .collect::<Cells>()
        })
        .collect::<Vec<_>>();
    let executor = InMemoryListExecutor {
        rows: rows.iter().cloned().zip(row_cells).collect(),
        enrich_page: None,
    };
    block_on(async {
        let count = executor.count(&where_).await;
        let plan = QueryPlan {
            where_,
            order: order.to_vec(),
            skip: 0,
            take: u64::try_from(rows.len()).unwrap_or(u64::MAX),
        };
        let selected = executor.find_many(&plan).await;
        let distinct = match distinct_field {
            Some(field) => executor.distinct(field, &plan.where_).await,
            None => Vec::new(),
        };
        Ok(json!({"count": count, "rows": selected, "distinct": distinct}))
    })?
}

fn filter_json(value: &FilterValue) -> Value {
    match value {
        FilterValue::Text(text) => Value::String(text.clone()),
        FilterValue::Int(number) => json!(number),
        FilterValue::Money(amount) => json!(amount.to_string()),
        FilterValue::Timestamp(timestamp) => json!(timestamp.to_rfc3339()),
    }
}

fn block_on<F>(future: F) -> Result<F::Output, Box<crate::helpers::HelperError>>
where
    F: Future,
{
    tokio::runtime::Builder::new_current_thread()
        .build()
        .map(|runtime| runtime.block_on(future))
        .map_err(|error| helper_error!("RuntimeError", error.to_string()))
}
