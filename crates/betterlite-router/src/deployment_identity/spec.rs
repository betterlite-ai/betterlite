use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::error::InvalidDeployment;

// The pricing fields Deployment.__init__ copies from litellm_params into model_info.
const MIRRORED_PRICING_FIELDS: [&str; 7] = [
    "input_cost_per_token",
    "output_cost_per_token",
    "input_cost_per_character",
    "output_cost_per_character",
    "cache_read_input_token_cost",
    "cache_creation_input_token_cost",
    "tiered_pricing",
];

// A built deployment: the fields of LiteLLM's Deployment with every null left out, as to_json(exclude_none=True) gives them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeploymentSpec {
    pub model_name: String,
    pub litellm_params: JsonObject,
    pub model_info: JsonObject,
    pub extra: JsonObject,
}

impl DeploymentSpec {
    // Deployment.__init__: model_info gets an id (new_id when it has none), its false defaults, and the mirrored pricing.
    pub fn new(
        model_name: String,
        litellm_params: JsonObject,
        mut model_info: JsonObject,
        extra: JsonObject,
        new_id: fn() -> String,
    ) -> Result<Self, InvalidDeployment> {
        let id = match model_info.get("id") {
            None | Some(Json::Null) => Json::Text(new_id()),
            Some(Json::Number(number)) if number.is_i64() || number.is_u64() => {
                Json::Text(number.to_string())
            }
            Some(Json::Text(text)) => Json::Text(text.clone()),
            Some(_) => return Err(InvalidDeployment::ModelInfoId),
        };
        model_info.insert("id".to_owned(), id);
        for flag in ["db_model", "member_auto_router"] {
            model_info
                .entry(flag.to_owned())
                .or_insert(Json::Bool(false));
        }
        for field in MIRRORED_PRICING_FIELDS {
            if let Some(rate) = litellm_params.get(field) {
                model_info.insert(field.to_owned(), rate.clone());
            }
        }
        model_info.retain(|_, value| !matches!(value, Json::Null));
        Ok(Self {
            model_name,
            litellm_params,
            model_info,
            extra,
        })
    }

    // The deployment id: model_info always carries one once the spec is built.
    #[must_use]
    pub fn id(&self) -> Option<&str> {
        match self.model_info.get("id") {
            Some(Json::Text(id)) => Some(id),
            _ => None,
        }
    }

    #[must_use]
    pub fn to_json(&self) -> JsonObject {
        let mut dumped = JsonObject::new();
        dumped.insert("model_name".to_owned(), Json::Text(self.model_name.clone()));
        dumped.insert(
            "litellm_params".to_owned(),
            Json::Object(self.litellm_params.clone()),
        );
        dumped.insert(
            "model_info".to_owned(),
            Json::Object(self.model_info.clone()),
        );
        for (key, value) in &self.extra {
            if !matches!(value, Json::Null) {
                dumped.insert(key.clone(), value.clone());
            }
        }
        dumped
    }
}
