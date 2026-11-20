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
    pub fn field(self) -> &'static str {
        match self {
            Self::ParallelFunctionCalling => "supports_parallel_function_calling",
            Self::Vision => "supports_vision",
            Self::WebSearch => "supports_web_search",
            Self::UrlContext => "supports_url_context",
            Self::Reasoning => "supports_reasoning",
            Self::FunctionCalling => "supports_function_calling",
        }
    }

    pub const ALL: [Self; 6] = [
        Self::ParallelFunctionCalling,
        Self::Vision,
        Self::WebSearch,
        Self::UrlContext,
        Self::Reasoning,
        Self::FunctionCalling,
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
        fields.insert(
            "max_input_tokens".to_owned(),
            json_float(self.max_input_tokens.as_ref()),
        );
        fields.insert(
            "max_output_tokens".to_owned(),
            json_float(self.max_output_tokens.as_ref()),
        );
        fields.insert(
            "input_cost_per_token".to_owned(),
            json_float(self.input_cost_per_token.as_ref()),
        );
        fields.insert(
            "output_cost_per_token".to_owned(),
            json_float(self.output_cost_per_token.as_ref()),
        );
        fields.insert(
            "input_cost_per_pixel".to_owned(),
            json_float(self.input_cost_per_pixel.as_ref()),
        );
        let mode = self.mode.clone().map(Json::Text).unwrap_or(Json::Null);
        fields.insert("mode".to_owned(), mode);
        let int = |value: Option<i64>| value.map(Number::from).map_or(Json::Null, Json::Number);
        fields.insert("tpm".to_owned(), int(self.tpm));
        fields.insert("rpm".to_owned(), int(self.rpm));
        fields.insert("itpm".to_owned(), int(self.itpm));
        fields.insert("otpm".to_owned(), int(self.otpm));
        for capability in Capability::ALL {
            let enabled = self.capabilities.contains(&capability);
            fields.insert(capability.field().to_owned(), Json::Bool(enabled));
        }
        fields.insert(
            "supports_fast_mode".to_owned(),
            Json::Bool(self.supports_fast_mode),
        );
        fields.insert(
            "supported_reasoning_efforts".to_owned(),
            json_efforts(self.supported_reasoning_efforts.as_deref()),
        );
        fields.insert(
            "supported_openai_params".to_owned(),
            self.supported_openai_params
                .as_deref()
                .map_or(Json::Null, text_list),
        );
        fields.insert(
            "configurable_clientside_auth_params".to_owned(),
            json_auth(self.clientside_auth.as_deref()),
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
    Json::List(
        values
            .iter()
            .map(|value| Json::Text(value.clone()))
            .collect(),
    )
}

fn json_efforts(efforts: Option<&[ReasoningEffort]>) -> Json {
    efforts.map_or(Json::Null, |efforts| {
        Json::List(
            efforts
                .iter()
                .map(|effort| Json::Text(effort_name(*effort).to_owned()))
                .collect(),
        )
    })
}

fn json_auth(entries: Option<&[ClientsideAuth]>) -> Json {
    entries.map_or(Json::Null, |entries| {
        Json::List(entries.iter().map(ClientsideAuth::to_json).collect())
    })
}

fn effort_name(effort: ReasoningEffort) -> &'static str {
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
