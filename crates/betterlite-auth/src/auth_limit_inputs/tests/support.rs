use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::{Arc, Mutex};

use betterlite_db::budget_permission_rows::JsonData;
use serde_json::Number;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::{Context, SubscriberExt as _};
use tracing_subscriber::registry;

use crate::auth_limit_inputs::deployment::{
    DeploymentLimitCatalog, DeploymentLimitValue, DeploymentLimits,
};
use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};

pub(crate) fn fields(entries: &[(&str, JsonData)]) -> BTreeMap<String, JsonData> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

pub(crate) fn object(entries: &[(&str, JsonData)]) -> JsonData {
    JsonData::Object(fields(entries))
}

pub(crate) fn number_object(entries: &[(&str, i64)]) -> JsonData {
    let fields: BTreeMap<String, JsonData> = entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), JsonData::Number(Number::from(*value))))
        .collect();
    JsonData::Object(fields)
}

pub(crate) fn number(value: i64) -> JsonData {
    JsonData::Number(Number::from(value))
}

pub(crate) fn float(value: f64) -> JsonData {
    JsonData::Number(Number::from_f64(value).unwrap())
}

pub(crate) fn text(value: &str) -> JsonData {
    JsonData::String(value.to_owned())
}

pub(crate) fn limits(entries: &[(&str, i64)]) -> BTreeMap<String, i64> {
    entries
        .iter()
        .map(|(model, limit)| ((*model).to_owned(), *limit))
        .collect()
}

pub(crate) fn auth(entries: &[(&str, JsonData)]) -> UserAPIKeyAuth {
    UserAPIKeyAuth::construct(AuthInput::Dict(fields(entries))).unwrap()
}

pub(crate) struct Catalog {
    pub(crate) model_name: String,
    pub(crate) deployments: Option<Vec<DeploymentLimits>>,
}

impl DeploymentLimitCatalog for Catalog {
    fn model_list(&self, model_name: &str) -> Option<Vec<DeploymentLimits>> {
        let deployments = self.deployments.as_ref()?;
        if model_name == self.model_name {
            return Some(deployments.clone());
        }
        Some(Vec::new())
    }
}

pub(crate) fn catalog(model_name: &str, deployments: Vec<DeploymentLimits>) -> Catalog {
    Catalog {
        model_name: model_name.to_owned(),
        deployments: Some(deployments),
    }
}

pub(crate) fn no_router() -> Catalog {
    Catalog {
        model_name: String::new(),
        deployments: None,
    }
}

pub(crate) fn deployment(
    rpm: Option<DeploymentLimitValue>,
    tpm: Option<DeploymentLimitValue>,
) -> DeploymentLimits {
    DeploymentLimits {
        default_api_key_rpm_limit: rpm,
        default_api_key_tpm_limit: tpm,
    }
}

#[derive(Clone, Default)]
struct WarningLog(Arc<Mutex<Vec<String>>>);

impl<S: Subscriber> Layer<S> for WarningLog {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        let mut visitor = WarningFields::default();
        event.record(&mut visitor);
        if let Some(field) = visitor.0
            && let Ok(mut warnings) = self.0.lock()
        {
            warnings.push(field);
        }
    }
}

#[derive(Default)]
struct WarningFields(Option<String>);

impl Visit for WarningFields {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "field" {
            self.0 = Some(value.to_owned());
        }
    }

    fn record_debug(&mut self, _field: &Field, _value: &dyn Debug) {}
}

pub(crate) fn collect_warnings<R, F>(run: F) -> (R, Vec<String>)
where
    F: FnOnce() -> R,
{
    let captured = WarningLog::default();
    let subscriber = registry().with(captured.clone());
    let result = tracing::subscriber::with_default(subscriber, run);
    let warnings = match captured.0.lock() {
        Ok(warnings) => warnings.clone(),
        Err(_) => Vec::new(),
    };
    (result, warnings)
}
