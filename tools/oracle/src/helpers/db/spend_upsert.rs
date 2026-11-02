use betterlite_db::autorouter_turn_sql::{
    sql::upsert_autorouter_session_sql,
    transaction::{AutoRouterTurnTransaction, upsert_params},
};
use betterlite_db::daily_spend_sql::{
    builder::build_bulk_upsert,
    entities::{DailySpendEntity, DailySpendEntityPayload, SpendRow, daily_spend_entity_ids},
    merge::merge_by_conflict_key,
};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt::Display;

pub(crate) struct Op;
#[derive(Deserialize)]
pub(crate) struct Input {
    steps: BTreeMap<String, Step>,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Step {
    Extract {
        entity: DailySpendEntity,
        payload: SpendRow,
    },
    Daily {
        entity: DailySpendEntity,
        rows: Vec<SpendRow>,
    },
    Turn {
        transaction: Box<AutoRouterTurnTransaction>,
    },
}
impl crate::helpers::Helper for Op {
    type Input = Input;
    fn run(input: &Input) -> crate::helpers::HelperResult {
        let mut output = BTreeMap::new();
        for (name, step) in &input.steps {
            let value = match step {
                Step::Extract { entity, payload } => {
                    let ids = serde_json::from_str::<DailySpendEntityPayload>(
                        &serde_json::to_string(payload).map_err(failure)?,
                    )
                    .ok()
                    .and_then(|record| daily_spend_entity_ids(&record, *entity).ok());
                    ids.map_or_else(
                        || serde_json::json!({"invalid": true}),
                        |entities| serde_json::json!({"ids": entities}),
                    )
                }
                Step::Daily { entity, rows } => daily(*entity, rows)?,
                Step::Turn { transaction } => serde_json::json!({
                    "sql": upsert_autorouter_session_sql().split_whitespace().collect::<Vec<_>>().join(" "),
                    "params": upsert_params(transaction),
                }),
            };
            output.insert(name, value);
        }
        serde_json::to_value(output).map_err(failure)
    }
}
fn daily(
    entity: DailySpendEntity,
    rows: &[SpendRow],
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let table = entity.table();
    let groups = merge_by_conflict_key(&table, rows).map_err(failure)?;
    let (sql, params) = build_bulk_upsert(&table, &groups).map_err(failure)?;
    Ok(
        serde_json::json!({"groups": groups, "sql": sql.split_whitespace().collect::<Vec<_>>().join(" "), "params": params}),
    )
}
fn failure<E>(error: E) -> Box<crate::helpers::HelperError>
where
    E: Display,
{
    Box::new(crate::helpers::HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
