use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::alerting::model_alerts::model_added::{
    RawModelInfoSource, render_model_added_message,
};
use betterlite_server::alerting::model_alerts::model_deprecation::{
    deprecation_alert_level, deprecation_alert_metadata,
};
use betterlite_server::model_deprecation::types::{
    DEFAULT_DEPRECATION_WARN_DAYS, DeprecationStatus, ModelDeprecationInfo,
    ModelDeprecationResponse,
};
use chrono::{DateTime, NaiveDate, Utc};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::json;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    ModelAdded {
        model_name: String,
        litellm_model_name: String,
        base_model: Option<String>,
        model_info: IndexMap<String, Json>,
        proxy_base_url: Option<String>,
    },
    DeprecationAlert {
        deprecated: usize,
        imminent: usize,
        upcoming: usize,
    },
}

struct CaseCatalog<'entries> {
    entries: &'entries IndexMap<String, Json>,
}

impl RawModelInfoSource for CaseCatalog<'_> {
    fn model_info(&self, name: &str) -> Option<&JsonObject> {
        match self.entries.get(name) {
            Some(Json::Object(fields)) => Some(fields),
            Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
            | None => None,
        }
    }
}

fn bucket(count: usize, status: DeprecationStatus) -> Vec<ModelDeprecationInfo> {
    (0..count)
        .map(|index| ModelDeprecationInfo {
            model_name: format!("model-{index}"),
            litellm_model: None,
            deprecation_date: NaiveDate::default(),
            days_until_deprecation: 0,
            status,
            litellm_provider: None,
        })
        .collect()
}

fn snapshot(deprecated: usize, imminent: usize, upcoming: usize) -> ModelDeprecationResponse {
    ModelDeprecationResponse {
        deprecated: bucket(deprecated, DeprecationStatus::Deprecated),
        imminent: bucket(imminent, DeprecationStatus::Imminent),
        upcoming: bucket(upcoming, DeprecationStatus::Upcoming),
        warn_within_days: DEFAULT_DEPRECATION_WARN_DAYS,
        checked_at: DateTime::<Utc>::UNIX_EPOCH,
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ModelAdded {
                model_name,
                litellm_model_name,
                base_model,
                model_info,
                proxy_base_url,
            } => {
                let catalog = CaseCatalog {
                    entries: model_info,
                };
                let message = render_model_added_message(
                    model_name,
                    litellm_model_name,
                    base_model.as_deref(),
                    &catalog,
                    proxy_base_url.as_deref(),
                )
                .or_value_error()?;
                Ok(json!({ "message": message }))
            }
            Input::DeprecationAlert {
                deprecated,
                imminent,
                upcoming,
            } => {
                if *deprecated == 0 && *imminent == 0 {
                    return Ok(json!({ "level": null, "metadata": null }));
                }
                let response = snapshot(*deprecated, *imminent, *upcoming);
                Ok(json!({
                    "level": deprecation_alert_level(&response).as_str(),
                    "metadata": deprecation_alert_metadata(&response),
                }))
            }
        }
    }
}

mod tests;
