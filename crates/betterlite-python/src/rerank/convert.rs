use betterlite_core::api::rerank_call::AsyncRerankArgs;
use betterlite_core::api::rerank_provider_plan::call::{RerankCall, ReturnDocuments};
use betterlite_error::error::{ErrorClass, class_message};
use betterlite_models::decisions::{HiddenParams, Json};
use betterlite_openai::api::rerank_contracts::records::{RerankDocument, RerankResponse};
use betterlite_openai::openai_base_config::Headers;
use betterlite_sdk::call_wrapper::error::{class_name, class_status};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::Number;
use std::collections::BTreeMap;
use uuid::Uuid;

mod tests;

// The one text a native method takes: the call's arguments, the remaining kwargs and the settings snapshot.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct RerankCallWire {
    pub(crate) arguments: RerankArguments,
    pub(crate) kwargs: IndexMap<String, Json>,
    pub(crate) settings: RerankSettingsWire,
}

// rerank's named arguments as main.py forwarded them; documents keep the strings and objects they arrived as.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct RerankArguments {
    pub(crate) model: String,
    pub(crate) query: String,
    pub(crate) documents: Vec<RerankDocument>,
    pub(crate) custom_llm_provider: Option<String>,
    pub(crate) top_n: Option<i64>,
    pub(crate) rank_fields: Option<Vec<String>>,
    pub(crate) return_documents: Option<bool>,
    pub(crate) max_chunks_per_doc: Option<i64>,
    pub(crate) max_tokens_per_doc: Option<i64>,
}

// the module-level values main.py's rerank reads before it dispatches
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct RerankSettingsWire {
    pub(crate) api_base: Option<String>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) cohere_key: Option<SecretString>,
    pub(crate) drop_params: Option<bool>,
    pub(crate) headers: Option<Headers>,
    pub(crate) togetherai_api_key: Option<SecretString>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct SessionWire {
    // absent means the process environment, the snapshot LiteLLM reads its globals from at import
    pub(crate) environment: Option<BTreeMap<String, String>>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ConvertError {
    #[error("the native request text is not JSON of the expected shape: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the answer could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}

// The JSON text NativeRerankError carries: LiteLLM's exception name and the fields its class keeps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct NativeRerankErrorPayload {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) message: String,
    pub(crate) status_code: Option<u16>,
    pub(crate) llm_provider: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) body: Option<Json>,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) num_retries: Option<i64>,
    pub(crate) max_retries: Option<i64>,
}

impl NativeRerankErrorPayload {
    #[must_use]
    pub(crate) fn from_class(class: &ErrorClass) -> Self {
        let metadata = class.metadata();
        let mut headers = BTreeMap::new();
        if let ErrorClass::RateLimit(details) = class {
            headers.clone_from(&details.headers);
        }
        Self {
            kind: class_name(class).to_owned(),
            message: class_message(class),
            // a plain Exception carries no status_code; every LiteLLM error class does
            status_code: class_status(class),
            llm_provider: metadata.provider,
            model: metadata.model,
            body: None,
            headers,
            num_retries: metadata.retry.num_retries.map(i64::from),
            max_retries: metadata.retry.max_retries.map(i64::from),
        }
    }
}

impl RerankCallWire {
    // rerank's own call: the ninth default (return_documents=True) arrives as a value from main.py
    pub(crate) fn into_call(self) -> RerankCall {
        let RerankArguments {
            model,
            query,
            documents,
            custom_llm_provider,
            top_n,
            rank_fields,
            return_documents,
            max_chunks_per_doc,
            max_tokens_per_doc,
        } = self.arguments;
        RerankCall {
            model,
            query,
            documents,
            custom_llm_provider,
            top_n,
            rank_fields,
            return_documents: return_documents
                .map_or(ReturnDocuments::Null, ReturnDocuments::Value),
            max_chunks_per_doc,
            max_tokens_per_doc,
            kwargs: self.kwargs,
        }
    }

    // arerank's call: its declared return_documents default is None, and max_tokens_per_doc stays a kwarg
    pub(crate) fn into_async_args(self) -> AsyncRerankArgs {
        AsyncRerankArgs {
            model: self.arguments.model,
            query: self.arguments.query,
            documents: self.arguments.documents,
            custom_llm_provider: self.arguments.custom_llm_provider,
            top_n: self.arguments.top_n,
            rank_fields: self.arguments.rank_fields,
            return_documents: self.arguments.return_documents,
            max_chunks_per_doc: self.arguments.max_chunks_per_doc,
            kwargs: self.kwargs,
        }
    }
}

// The logging object's values as set_hidden_params writes them: get_api_base only ever reads the call's kwargs.
pub(crate) struct HiddenFacts<'facts> {
    pub(crate) call_id: &'facts str,
    pub(crate) api_base: Option<&'facts str>,
    pub(crate) litellm_model: &'facts str,
    pub(crate) model_id: Option<&'facts Json>,
    pub(crate) provider: &'facts str,
    pub(crate) response_ms: f64,
    // arerank measures the wrapper's overhead as the response time its own transport did not spend
    pub(crate) overhead_ms: Option<f64>,
}

#[must_use]
pub(crate) fn hidden_params(facts: &HiddenFacts<'_>) -> HiddenParams {
    let mut hidden = HiddenParams::new();
    hidden.insert(
        String::from("_response_ms"),
        Number::from_f64(facts.response_ms).map_or(Json::Null, Json::Number),
    );
    hidden.insert(
        String::from("callback_duration_ms"),
        Number::from_f64(0.0_f64).map_or(Json::Null, Json::Number),
    );
    hidden.insert(
        String::from("litellm_call_id"),
        Json::Text(facts.call_id.to_owned()),
    );
    hidden.insert(
        String::from("api_base"),
        facts
            .api_base
            .map_or(Json::Null, |base| Json::Text(base.to_owned())),
    );
    hidden.insert(
        String::from("model_id"),
        facts.model_id.map_or(Json::Null, Clone::clone),
    );
    // main.py overwrites the cost with the priced answer, as LiteLLM's logging does after the call
    hidden.insert(String::from("response_cost"), Json::Null);
    hidden.insert(
        String::from("additional_headers"),
        Json::Object(IndexMap::new()),
    );
    hidden.insert(
        String::from("litellm_model_name"),
        Json::Text(facts.litellm_model.to_owned()),
    );
    hidden.insert(
        String::from("custom_llm_provider"),
        Json::Text(facts.provider.to_owned()),
    );
    if let Some(overhead) = facts.overhead_ms {
        hidden.insert(
            String::from("litellm_overhead_time_ms"),
            Number::from_f64(overhead).map_or(Json::Null, Json::Number),
        );
    }
    hidden
}

// The JSON text Python validates as RerankResponse, beside the provider the cost lookup needs.
#[derive(Debug, Serialize)]
pub(crate) struct RerankAnswer {
    #[serde(flatten)]
    pub(crate) response: RerankResponse,
    pub(crate) provider: String,
    pub(crate) hidden_params: HiddenParams,
    pub(crate) response_ms: f64,
}

// A kwarg read as text only when it holds a non-empty string, the one shape set_hidden_params keeps.
fn non_empty_text(value: Option<&Json>) -> Option<&str> {
    match value {
        Some(Json::Text(text)) if !text.is_empty() => Some(text),
        Some(
            Json::Text(_)
            | Json::Bool(_)
            | Json::Number(_)
            | Json::List(_)
            | Json::Object(_)
            | Json::Null,
        )
        | None => None,
    }
}

// set_hidden_params reports get_api_base over the call's own kwargs, never the resolved deployment.
#[must_use]
pub(crate) fn declared_api_base(kwargs: &IndexMap<String, Json>) -> Option<String> {
    if let Some(Json::Text(base)) = kwargs.get("api_base") {
        return Some(base.clone());
    }
    non_empty_text(kwargs.get("base_url")).map(str::to_owned)
}

// set_hidden_params reads kwargs["model_info"]["id"] into the answer's model_id
pub(crate) fn model_info_id(kwargs: &IndexMap<String, Json>) -> Option<&Json> {
    let Json::Object(info) = kwargs.get("model_info")? else {
        return None;
    };
    info.get("id")
}

// `litellm_call_id = kwargs.get("litellm_call_id") or str(uuid.uuid4())`
#[must_use]
pub(crate) fn call_id(kwargs: &IndexMap<String, Json>) -> String {
    match non_empty_text(kwargs.get("litellm_call_id")) {
        Some(text) => text.to_owned(),
        None => Uuid::new_v4().to_string(),
    }
}
