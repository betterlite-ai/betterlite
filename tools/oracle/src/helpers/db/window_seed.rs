use std::collections::BTreeMap;
use std::fmt::Display;

use betterlite_db::window_queue::WindowSpendTransaction;
use betterlite_db::window_spend::seed::{
    WindowSeedTotals, exclusion_upper_bound, seed_base, window_instant,
};
use betterlite_db::window_spend::upsert::upsert_params;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use chrono::NaiveDateTime;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Step {
    SeedBase {
        name: String,
        total: Decimal,
        before_batch: Decimal,
        batch_spend: Decimal,
    },
    ExclusionUpperBound {
        name: String,
        started_at: String,
    },
    UpsertParams {
        name: String,
        transaction: WindowSpendTransaction,
        seed_base: Decimal,
        now: String,
    },
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut results = BTreeMap::new();
        for step in &input.steps {
            let (name, value) = result_for(step)?;
            results.insert(name, value);
        }
        serde_json::to_value(results).map_err(failure)
    }
}

fn result_for(step: &Step) -> Result<(String, Value), Box<HelperError>> {
    match step {
        Step::SeedBase {
            name,
            total,
            before_batch,
            batch_spend,
        } => Ok((
            name.clone(),
            json!(seed_amount(*total, *before_batch, *batch_spend)?),
        )),
        Step::ExclusionUpperBound { name, started_at } => {
            Ok((name.clone(), json!(floored_instant(started_at)?)))
        }
        Step::UpsertParams {
            name,
            transaction,
            seed_base,
            now,
        } => Ok((name.clone(), params_value(transaction, *seed_base, now)?)),
    }
}

fn seed_amount(
    total: Decimal,
    before_batch: Decimal,
    batch_spend: Decimal,
) -> Result<String, Box<HelperError>> {
    let totals = WindowSeedTotals {
        total,
        before_batch,
    };
    let base = seed_base(&totals, batch_spend).map_err(failure)?;
    Ok(format!("{base:.6}"))
}

fn floored_instant(started_at: &str) -> Result<String, Box<HelperError>> {
    let instant = window_instant(started_at).map_err(failure)?;
    Ok(isoformat(exclusion_upper_bound(instant)))
}

fn params_value(
    transaction: &WindowSpendTransaction,
    seed_base: Decimal,
    now: &str,
) -> Result<Value, Box<HelperError>> {
    let instant = window_instant(now).map_err(failure)?;
    let params = upsert_params(transaction, seed_base, instant).map_err(failure)?;
    Ok(json!({
        "entity_type": params.entity_type,
        "entity_id": params.entity_id,
        "window_duration": params.window_duration,
        "window_start": isoformat(params.window_start),
        "spend": format!("{:.6}", params.spend),
        "increment": format!("{:.6}", params.increment),
        "now": isoformat(params.now),
    }))
}

// Python's datetime.isoformat writes six fractional digits only when the microseconds are non-zero
fn isoformat(stamp: NaiveDateTime) -> String {
    let base = stamp.format("%Y-%m-%dT%H:%M:%S").to_string();
    match stamp.and_utc().timestamp_subsec_micros() {
        0 => base,
        micros => format!("{base}.{micros:06}"),
    }
}

fn failure<Error>(error: Error) -> Box<HelperError>
where
    Error: Display,
{
    Box::new(HelperError {
        class: "WindowSeed".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
