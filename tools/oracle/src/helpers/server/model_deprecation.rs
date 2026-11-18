use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::model_deprecation::alert::format_alert_message;
use betterlite_server::model_deprecation::catalog::{CatalogDeprecation, DeprecationCatalog};
use betterlite_server::model_deprecation::clock::UtcClock;
use betterlite_server::model_deprecation::dates::{decode_datetime_date, decode_deprecation_date};
use betterlite_server::model_deprecation::report::collect_model_deprecations;
use betterlite_server::model_deprecation::resolve::Deployment;
use betterlite_server::model_deprecation::types::DEFAULT_DEPRECATION_WARN_DAYS;
use chrono::{DateTime, NaiveDate, Utc};
use serde::Deserialize;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    now: DateTime<Utc>,
    model_cost: BTreeMap<String, Json>,
    deployments: Option<Vec<Row>>,
    #[serde(default)]
    warn_within_days: Option<i32>,
    #[serde(default)]
    today: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Row {
    #[serde(default)]
    model_name: Option<Json>,
    #[serde(default)]
    model_info: Option<Json>,
    #[serde(default)]
    litellm_params: Option<Json>,
}

struct CaseCatalog<'entries> {
    entries: &'entries BTreeMap<String, Json>,
}

impl DeprecationCatalog for CaseCatalog<'_> {
    fn deprecation(&self, model_key: &str) -> Option<CatalogDeprecation> {
        let Some(Json::Object(fields)) = self.entries.get(model_key) else {
            return None;
        };
        Some(CatalogDeprecation {
            deprecation_date: tagged_date(fields.get("deprecation_date"))?,
            litellm_provider: text(fields.get("litellm_provider")).map(str::to_owned),
        })
    }
}

struct FixedClock(DateTime<Utc>);

impl UtcClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

fn text(raw: Option<&Json>) -> Option<&str> {
    match raw {
        Some(Json::Text(value)) => Some(value),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

const fn mapping(raw: Option<&Json>) -> Option<&JsonObject> {
    match raw {
        Some(Json::Object(fields)) => Some(fields),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => None,
    }
}

fn marker_date(fields: &JsonObject) -> Option<NaiveDate> {
    match text(fields.get("kind"))? {
        "date" => decode_deprecation_date(text(fields.get("value"))?),
        "datetime" => decode_datetime_date(text(fields.get("value"))?),
        _ => None,
    }
}

fn tagged_date(raw: Option<&Json>) -> Option<NaiveDate> {
    match raw {
        Some(Json::Text(value)) => decode_deprecation_date(value),
        Some(Json::Object(fields)) => marker_date(fields),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_)) | None => None,
    }
}

fn deployment(row: &Row) -> Deployment<'_> {
    let model_info = mapping(row.model_info.as_ref());
    Deployment {
        model_name: text(row.model_name.as_ref()),
        model_info,
        litellm_params: mapping(row.litellm_params.as_ref()),
        override_date: model_info.and_then(|fields| tagged_date(fields.get("deprecation_date"))),
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let catalog = CaseCatalog {
            entries: &input.model_cost,
        };
        let deployments: Vec<Deployment<'_>> = input
            .deployments
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(deployment)
            .collect();
        let snapshot = collect_model_deprecations(
            &deployments,
            &catalog,
            &FixedClock(input.now),
            input
                .warn_within_days
                .unwrap_or(DEFAULT_DEPRECATION_WARN_DAYS),
            input.today.as_deref().and_then(decode_deprecation_date),
        );
        let message = format_alert_message(&snapshot);
        Ok(serde_json::json!({ "snapshot": snapshot, "message": message }))
    }
}
