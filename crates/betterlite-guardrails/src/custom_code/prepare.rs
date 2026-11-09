use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

// The five keys _prepare_safe_request_data hands to custom code; credentials stay out of reach.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct SafeRequestData {
    pub model: Option<GuardrailValue>,
    pub user_id: Option<GuardrailValue>,
    pub team_id: Option<GuardrailValue>,
    pub end_user_id: Option<GuardrailValue>,
    pub metadata: BTreeMap<String, GuardrailValue>,
}

impl SafeRequestData {
    #[must_use]
    pub(super) fn to_value(&self) -> GuardrailValue {
        let mut object = BTreeMap::new();
        object.insert(
            "model".to_owned(),
            self.model.clone().unwrap_or(GuardrailValue::Null),
        );
        object.insert(
            "user_id".to_owned(),
            self.user_id.clone().unwrap_or(GuardrailValue::Null),
        );
        object.insert(
            "team_id".to_owned(),
            self.team_id.clone().unwrap_or(GuardrailValue::Null),
        );
        object.insert(
            "end_user_id".to_owned(),
            self.end_user_id.clone().unwrap_or(GuardrailValue::Null),
        );
        object.insert(
            "metadata".to_owned(),
            GuardrailValue::Object(self.metadata.clone()),
        );
        GuardrailValue::Object(object)
    }
}

// LiteLLM merges metadata and litellm_metadata, later keys winning, and ignores non-object buckets.
#[must_use]
pub(super) fn merged_metadata(
    request_data: &BTreeMap<String, GuardrailValue>,
) -> BTreeMap<String, GuardrailValue> {
    let mut metadata = BTreeMap::new();
    for key in ["metadata", "litellm_metadata"] {
        if let Some(GuardrailValue::Object(bucket)) = request_data.get(key) {
            metadata.extend(bucket.clone());
        }
    }
    metadata
}
