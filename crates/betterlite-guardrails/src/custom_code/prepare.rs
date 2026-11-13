use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

// The five keys _prepare_safe_request_data hands to custom code; credentials stay out of reach.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct SafeRequestData {
    pub model: GuardrailValue,
    pub user_id: GuardrailValue,
    pub team_id: GuardrailValue,
    pub end_user_id: GuardrailValue,
    pub metadata: BTreeMap<String, GuardrailValue>,
}

impl SafeRequestData {
    #[must_use]
    pub(super) fn to_value(&self) -> GuardrailValue {
        GuardrailValue::Object(BTreeMap::from([
            ("model".to_owned(), self.model.clone()),
            ("user_id".to_owned(), self.user_id.clone()),
            ("team_id".to_owned(), self.team_id.clone()),
            ("end_user_id".to_owned(), self.end_user_id.clone()),
            (
                "metadata".to_owned(),
                GuardrailValue::Object(self.metadata.clone()),
            ),
        ]))
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
