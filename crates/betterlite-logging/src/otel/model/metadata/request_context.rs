use super::common::{field, object, string};
use super::request_identity::RequestIdentity;
use crate::otel::model::metadata::request_identity::parse::from_payload;
use crate::otel::model::utils::coercion::OtelModelValue;

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct RequestContext {
    pub request_model: String,
    pub response_model: Option<String>,
    pub model_group: Option<String>,
    pub model_id: Option<String>,
    pub api_base: Option<String>,
    pub identity: RequestIdentity,
}

impl RequestContext {
    #[must_use]
    pub fn from_standard_logging_payload(payload: &OtelModelValue) -> Self {
        let metadata = object(field(payload, "metadata"));
        let hidden = object(field(payload, "hidden_params"));
        let response = object(field(payload, "response"));
        let model_group = string(field(payload, "model_group"))
            .or_else(|| string(metadata.and_then(|values| values.get("model_group"))));
        let model_id = string(field(payload, "model_id")).or_else(|| {
            string(
                metadata
                    .and_then(|values| values.get("model_info"))
                    .and_then(|value| field(value, "id")),
            )
        });
        let request_model = model_group
            .clone()
            .or_else(|| string(field(payload, "model")))
            .unwrap_or_default();
        Self {
            request_model,
            response_model: string(response.and_then(|values| values.get("model"))),
            model_group,
            model_id,
            api_base: string(field(payload, "api_base"))
                .or_else(|| string(hidden.and_then(|values| values.get("api_base")))),
            identity: from_payload(payload),
        }
    }
}

mod tests;
