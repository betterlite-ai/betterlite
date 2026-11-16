use std::collections::{BTreeMap, BTreeSet};

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::daily_activity_sql::PTU_SENTINEL_API_KEY;
use betterlite_db::ptu::charges::decode::decode_ptu_model_info;
use betterlite_db::ptu::charges::deployment::{RouterEntry, config_deployments, parse_ptu_model};
use betterlite_db::ptu::charges::pricing::{
    active_hours_on_day, aggregate_charges, daily_flat_cost,
};
use betterlite_db::ptu::charges::window::{backfill_window, lapsed_models};
use betterlite_db::ptu::store::prune::{prune_chunks, prune_cutoff};
use betterlite_oracle_helper_api::error::HelperError;
use serde_json::{Value, json};

use self::values::{
    amount_value, charge_failure, charge_value, parsed_models, parsed_value, ptu_model,
    router_entry, store_failure, utc_day, utc_instant,
};

mod values;

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    steps: BTreeMap<String, Step>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Step {
    Decode {
        raw: JsonData,
    },
    Parse {
        model_id: String,
        model_name: String,
        model_info: JsonData,
    },
    Hours {
        model: ModelInput,
        day: String,
    },
    Cost {
        model: ModelInput,
        day: String,
    },
    Aggregate {
        models: Vec<ModelInput>,
        day: String,
    },
    Config {
        entries: Vec<EntryInput>,
        owned_by_db: Vec<String>,
    },
    Lapsed {
        models: Vec<ModelInput>,
        now_micros: i64,
    },
    Backfill {
        models: Vec<ModelInput>,
        end: String,
    },
    Prune {
        date: String,
        run_started_micros: i64,
        scanned_ids: Vec<String>,
    },
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelInput {
    model_id: String,
    model_name: String,
    team_id: String,
    ptu_count: u32,
    cost_per_ptu_per_hour: String,
    effective_from_micros: i64,
    effective_to_micros: Option<i64>,
}

#[derive(Debug, serde::Deserialize)]
struct EntryInput {
    #[serde(default)]
    model_name: String,
    model_info: Option<JsonData>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let mut answers: BTreeMap<String, Value> = BTreeMap::new();
        for (name, step) in &input.steps {
            answers.insert(name.clone(), answer(step)?);
        }
        Ok(serde_json::to_value(answers)?)
    }
}

fn answer(step: &Step) -> Result<Value, Box<HelperError>> {
    match step {
        Step::Decode { raw } => decode_answer(raw),
        Step::Parse {
            model_id,
            model_name,
            model_info,
        } => parse_answer(model_id, model_name, model_info),
        Step::Hours { model, day } => hours_answer(model, day),
        Step::Cost { model, day } => cost_answer(model, day),
        Step::Aggregate { models, day } => aggregate_answer(models, day),
        Step::Config {
            entries,
            owned_by_db,
        } => Ok(config_answer(entries, owned_by_db)),
        Step::Lapsed { models, now_micros } => lapsed_answer(models, *now_micros),
        Step::Backfill { models, end } => backfill_answer(models, end),
        Step::Prune {
            date,
            run_started_micros,
            scanned_ids,
        } => prune_answer(date, *run_started_micros, scanned_ids),
    }
}

fn decode_answer(raw: &JsonData) -> Result<Value, Box<HelperError>> {
    let Some(model_info) = decode_ptu_model_info(raw) else {
        return Ok(Value::Null);
    };
    Ok(serde_json::to_value(model_info.as_ref())?)
}

fn parse_answer(
    model_id: &str,
    model_name: &str,
    raw: &JsonData,
) -> Result<Value, Box<HelperError>> {
    let Some(model_info) = decode_ptu_model_info(raw) else {
        return Ok(Value::Null);
    };
    let Some(model) = parse_ptu_model(model_id, model_name, &model_info) else {
        return Ok(Value::Null);
    };
    parsed_value(&model)
}

fn hours_answer(model: &ModelInput, day: &str) -> Result<Value, Box<HelperError>> {
    let model = ptu_model(model)?;
    let hours = active_hours_on_day(&model, utc_day(day)?).map_err(charge_failure)?;
    amount_value(hours)
}

fn cost_answer(model: &ModelInput, day: &str) -> Result<Value, Box<HelperError>> {
    let model = ptu_model(model)?;
    let cost = daily_flat_cost(&model, utc_day(day)?).map_err(charge_failure)?;
    amount_value(cost)
}

fn aggregate_answer(models: &[ModelInput], day: &str) -> Result<Value, Box<HelperError>> {
    let models = parsed_models(models)?;
    let charges = aggregate_charges(&models, utc_day(day)?).map_err(charge_failure)?;
    let mut values: Vec<Value> = Vec::new();
    for charge in &charges {
        values.push(charge_value(charge)?);
    }
    Ok(Value::Array(values))
}

fn config_answer(entries: &[EntryInput], owned_by_db: &[String]) -> Value {
    let records: Vec<RouterEntry> = entries.iter().map(router_entry).collect();
    let owned: BTreeSet<String> = owned_by_db.iter().cloned().collect();
    let mut values: Vec<Value> = Vec::new();
    for deployment in config_deployments(&records, &owned) {
        values.push(json!({
            "model_id": deployment.model_id,
            "model_name": deployment.model_name,
        }));
    }
    Value::Array(values)
}

fn lapsed_answer(models: &[ModelInput], now_micros: i64) -> Result<Value, Box<HelperError>> {
    let models = parsed_models(models)?;
    let names = lapsed_models(&models, utc_instant(now_micros)?);
    Ok(json!(names))
}

fn backfill_answer(models: &[ModelInput], end: &str) -> Result<Value, Box<HelperError>> {
    let models = parsed_models(models)?;
    let days: Vec<String> = backfill_window(&models, utc_day(end)?)
        .iter()
        .map(ToString::to_string)
        .collect();
    Ok(json!(days))
}

fn prune_answer(
    date: &str,
    run_started_micros: i64,
    scanned_ids: &[String],
) -> Result<Value, Box<HelperError>> {
    let cutoff = prune_cutoff(utc_instant(run_started_micros)?).map_err(store_failure)?;
    let scanned: BTreeSet<String> = scanned_ids.iter().cloned().collect();
    let mut values: Vec<Value> = Vec::new();
    for chunk in prune_chunks(date, cutoff, &scanned) {
        let cutoff_micros = chunk.cutoff.timestamp_micros();
        values.push(json!({
            "date": chunk.date,
            "cutoff_micros": cutoff_micros,
            "api_key": PTU_SENTINEL_API_KEY,
            "models": chunk.models,
        }));
    }
    Ok(Value::Array(values))
}
