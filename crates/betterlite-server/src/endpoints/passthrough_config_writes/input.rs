use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::endpoints::passthrough_config_reads::record::EndpointFlag;
use crate::endpoints::passthrough_config_reads::record::PassThroughGenericEndpoint;
use crate::endpoints::passthrough_config_reads::record::PassThroughGuardrailSettings;

// LiteLLM's PassThroughGuardrailSettings defaults both field lists, so `{"pii": {}}` is a valid settings object
#[derive(Debug, Clone, Deserialize)]
pub struct GuardrailSettingsInput {
    #[serde(default)]
    pub request_fields: Option<Vec<String>>,
    #[serde(default)]
    pub response_fields: Option<Vec<String>>,
}

// LiteLLM's PassThroughGenericEndpoint body: path and target are required, every other key is optional
#[derive(Debug, Clone, Deserialize)]
pub struct EndpointInput {
    pub path: String,
    pub target: String,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub headers: Option<BTreeMap<String, JsonData>>,
    #[serde(default)]
    pub default_query_params: Option<BTreeMap<String, JsonData>>,
    #[serde(default)]
    pub include_subpath: Option<bool>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub cost_per_request: Option<Decimal>,
    #[serde(default)]
    pub timeout: Option<f64>,
    #[serde(default)]
    pub auth: Option<bool>,
    #[serde(default)]
    pub guardrails: Option<BTreeMap<String, Option<GuardrailSettingsInput>>>,
    #[serde(default)]
    pub methods: Option<Vec<String>>,
}

fn guardrail_settings(settings: GuardrailSettingsInput) -> PassThroughGuardrailSettings {
    PassThroughGuardrailSettings {
        request_fields: settings.request_fields,
        response_fields: settings.response_fields,
    }
}

fn guardrail_records(
    guardrails: BTreeMap<String, Option<GuardrailSettingsInput>>,
) -> BTreeMap<String, Option<PassThroughGuardrailSettings>> {
    guardrails
        .into_iter()
        .map(|(name, settings)| (name, settings.map(guardrail_settings)))
        .collect()
}

impl EndpointInput {
    // the sent fields over a base entry: the stored one on update, the body model's defaults on create
    pub(super) fn merge_into(
        &self,
        stored: &PassThroughGenericEndpoint,
    ) -> PassThroughGenericEndpoint {
        PassThroughGenericEndpoint {
            id: self.id.clone().or_else(|| stored.id.clone()),
            path: self.path.clone(),
            target: self.target.clone(),
            headers: self
                .headers
                .clone()
                .unwrap_or_else(|| stored.headers.clone()),
            default_query_params: self
                .default_query_params
                .clone()
                .unwrap_or_else(|| stored.default_query_params.clone()),
            include_subpath: EndpointFlag(self.include_subpath.unwrap_or(stored.include_subpath.0)),
            cost_per_request: self.cost_per_request.unwrap_or(stored.cost_per_request),
            timeout: self.timeout.or(stored.timeout),
            auth: EndpointFlag(self.auth.unwrap_or(stored.auth.0)),
            guardrails: self
                .guardrails
                .clone()
                .map(guardrail_records)
                .or_else(|| stored.guardrails.clone()),
            is_from_config: EndpointFlag(false),
            methods: self.methods.clone().or_else(|| stored.methods.clone()),
        }
    }
}
