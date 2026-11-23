use betterlite_error::error::{ErrorClass, class_message};
use betterlite_models::decisions::Json;
use betterlite_openai::embedding::EmbeddingInput;
use betterlite_openai::openai_base_config::Headers;
use betterlite_sdk::call_wrapper::error::{class_name, class_status};
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
mod tests;
// The one text a native method takes: the call's arguments and the settings snapshot main.py read.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct EmbeddingCallWire {
    pub(crate) arguments: EmbeddingArguments,
    pub(crate) settings: EmbeddingSettingsWire,
}
// main.py's named arguments, the merged headers and every other kwarg with the shape it arrived in.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct EmbeddingArguments {
    pub(crate) model: String,
    pub(crate) input: Option<EmbeddingInput>,
    pub(crate) dimensions: Option<u32>,
    pub(crate) encoding_format: Option<String>,
    pub(crate) timeout: Option<f64>,
    pub(crate) user: Option<String>,
    pub(crate) api_base: Option<String>,
    pub(crate) api_version: Option<String>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) custom_llm_provider: Option<String>,
    pub(crate) litellm_call_id: Option<String>,
    pub(crate) headers: Option<Headers>,
    pub(crate) mock_response: Option<Json>,
    pub(crate) input_cost_per_token: Option<Decimal>,
    pub(crate) output_cost_per_token: Option<Decimal>,
    pub(crate) allowed_openai_params: Option<Vec<String>>,
    pub(crate) additional_drop_params: Option<Vec<String>>,
    pub(crate) drop_params: Option<bool>,
    #[serde(flatten)]
    pub(crate) extra: BTreeMap<String, Json>,
}
// the module-level values main.py's embedding reads before it dispatches
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct EmbeddingSettingsWire {
    pub(crate) api_base: Option<String>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) openai_key: Option<SecretString>,
    pub(crate) drop_params: Option<bool>,
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
// The JSON text NativeEmbeddingError carries: LiteLLM's exception name and the fields its class keeps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct NativeEmbeddingErrorPayload {
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
impl NativeEmbeddingErrorPayload {
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
