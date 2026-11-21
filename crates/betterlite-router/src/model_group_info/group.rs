use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use betterlite_models::metadata::levels::ReasoningEffort;
use betterlite_models::model_registration::JsonObject;
use rust_decimal::Decimal;
use serde_json::Number;

use crate::model_group_info::fields::json_float;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Capability {
    ParallelFunctionCalling,
    Vision,
    WebSearch,
    UrlContext,
    Reasoning,
    FunctionCalling,
}

impl Capability {
    pub const ALL: [(&'static str, Self); 6] = [
        (
            "supports_parallel_function_calling",
            Self::ParallelFunctionCalling,
        ),
        ("supports_vision", Self::Vision),
        ("supports_web_search", Self::WebSearch),
        ("supports_url_context", Self::UrlContext),
        ("supports_reasoning", Self::Reasoning),
        ("supports_function_calling", Self::FunctionCalling),
    ];
}

// One configurable_clientside_auth_params item: a parameter name, or an api_base object the constructor validates.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClientsideAuth {
    Name(String),
    Item(JsonObject),
}

// types/router.py ModelGroupInfo: the aggregate one model group answers with.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelGroupInfo {
    pub model_group: String,
    pub providers: Vec<String>,
    pub max_input_tokens: Option<Decimal>,
    pub max_output_tokens: Option<Decimal>,
    pub input_cost_per_token: Option<Decimal>,
    pub output_cost_per_token: Option<Decimal>,
    pub input_cost_per_pixel: Option<Decimal>,
    pub mode: Option<String>,
    pub tpm: Option<i64>,
    pub rpm: Option<i64>,
    pub itpm: Option<i64>,
    pub otpm: Option<i64>,
    pub capabilities: BTreeSet<Capability>,
    pub supports_fast_mode: bool,
    pub supported_reasoning_efforts: Option<Vec<ReasoningEffort>>,
    pub supported_openai_params: Option<Vec<String>>,
    pub clientside_auth: Option<Vec<ClientsideAuth>>,
}

impl ModelGroupInfo {
    #[must_use]
    pub fn to_json(&self) -> JsonObject {
        let mut fields = JsonObject::new();
        fields.insert(
            "model_group".to_owned(),
            Json::Text(self.model_group.clone()),
        );
        fields.insert("providers".to_owned(), text_list(&self.providers));
        for (field, cost) in [
            ("max_input_tokens", &self.max_input_tokens),
            ("max_output_tokens", &self.max_output_tokens),
            ("input_cost_per_token", &self.input_cost_per_token),
            ("output_cost_per_token", &self.output_cost_per_token),
            ("input_cost_per_pixel", &self.input_cost_per_pixel),
        ] {
            fields.insert(field.to_owned(), json_float(cost.as_ref()));
        }
        fields.insert(
            "mode".to_owned(),
            self.mode.clone().map_or(Json::Null, Json::Text),
        );
        for (field, amount) in [
            ("tpm", self.tpm),
            ("rpm", self.rpm),
            ("itpm", self.itpm),
            ("otpm", self.otpm),
        ] {
            let value = amount.map(Number::from).map_or(Json::Null, Json::Number);
            fields.insert(field.to_owned(), value);
        }
        fields.insert(
            "supports_fast_mode".to_owned(),
            Json::Bool(self.supports_fast_mode),
        );
        for (field, capability) in Capability::ALL {
            let enabled = self.capabilities.contains(&capability);
            fields.insert(field.to_owned(), Json::Bool(enabled));
        }
        fields.insert(
            "supported_reasoning_efforts".to_owned(),
            json_list(self.supported_reasoning_efforts.as_deref(), |effort| {
                Json::Text(effort_name(*effort).to_owned())
            }),
        );
        fields.insert(
            "supported_openai_params".to_owned(),
            self.supported_openai_params
                .as_deref()
                .map_or(Json::Null, text_list),
        );
        fields.insert(
            "configurable_clientside_auth_params".to_owned(),
            json_list(self.clientside_auth.as_deref(), ClientsideAuth::to_json),
        );
        fields
    }
}

impl ClientsideAuth {
    #[must_use]
    pub fn to_json(&self) -> Json {
        match self {
            Self::Name(name) => Json::Text(name.clone()),
            Self::Item(fields) => Json::Object(fields.clone()),
        }
    }
}

fn text_list(values: &[String]) -> Json {
    Json::List(values.iter().cloned().map(Json::Text).collect())
}

fn json_list<T>(values: Option<&[T]>, convert: fn(&T) -> Json) -> Json {
    values.map_or(Json::Null, |items| {
        Json::List(items.iter().map(convert).collect())
    })
}

const fn effort_name(effort: ReasoningEffort) -> &'static str {
    match effort {
        ReasoningEffort::None => "none",
        ReasoningEffort::Minimal => "minimal",
        ReasoningEffort::Low => "low",
        ReasoningEffort::Medium => "medium",
        ReasoningEffort::High => "high",
        ReasoningEffort::Xhigh => "xhigh",
        ReasoningEffort::Max => "max",
    }
}
