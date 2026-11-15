use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::guardrail_base::GuardrailBase;

// LiteLLM's PRE_CALL_EXECUTED_GUARDRAILS_KEY; the name limit drops GUARDRAILS
pub const PRE_CALL_EXECUTED_KEY: &str = "_pre_call_executed_guardrails";

// The per-process secret that tags every pre-call marker; startup makes one from Uuid::new_v4().simple(), 32 lowercase hex like secrets.token_hex(16)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreCallToken(pub String);

// The crate keeps one inherent impl block per type, so these methods hang off a trait
pub trait PreCallMarkers {
    fn pre_call_marker(&self, token: &PreCallToken) -> Option<String>;

    fn mark_pre_call_hook_ran(
        &self,
        token: &PreCallToken,
        data: &mut BTreeMap<String, GuardrailValue>,
    );

    fn pre_call_hook_already_ran(
        &self,
        token: &PreCallToken,
        data: &BTreeMap<String, GuardrailValue>,
    ) -> bool;
}

impl PreCallMarkers for GuardrailBase {
    // LiteLLM's _pre_call_marker; an empty name is used as given
    #[must_use]
    fn pre_call_marker(&self, token: &PreCallToken) -> Option<String> {
        self.settings
            .guardrail_name
            .as_ref()
            .map(|name| format!("{}:{name}", token.0))
    }

    // Records that this guardrail's pre-call hook ran, in the first of metadata and litellm_metadata that is an object
    fn mark_pre_call_hook_ran(
        &self,
        token: &PreCallToken,
        data: &mut BTreeMap<String, GuardrailValue>,
    ) {
        let Some(text) = self.pre_call_marker(token) else {
            return;
        };
        let marker = GuardrailValue::Text(text);
        let found = ["metadata", "litellm_metadata"]
            .into_iter()
            .find(|name| matches!(data.get(*name), Some(GuardrailValue::Object(_))));
        let location = found.unwrap_or("metadata");
        if found.is_none() {
            data.insert(
                "metadata".to_owned(),
                GuardrailValue::Object(BTreeMap::new()),
            );
        }
        let Some(GuardrailValue::Object(bucket)) = data.get_mut(location) else {
            return;
        };
        let executed = bucket
            .entry(PRE_CALL_EXECUTED_KEY.to_owned())
            .or_insert_with(|| GuardrailValue::Array(Vec::new()));
        if !matches!(executed, GuardrailValue::Array(_)) {
            *executed = GuardrailValue::Array(Vec::new());
        }
        if let GuardrailValue::Array(list) = executed
            && !list.contains(&marker)
        {
            list.push(marker);
        }
    }

    // True when the marker is in the list of either bucket
    #[must_use]
    fn pre_call_hook_already_ran(
        &self,
        token: &PreCallToken,
        data: &BTreeMap<String, GuardrailValue>,
    ) -> bool {
        let Some(text) = self.pre_call_marker(token) else {
            return false;
        };
        let marker = GuardrailValue::Text(text);
        ["metadata", "litellm_metadata"]
            .into_iter()
            .any(|location| match data.get(location) {
                Some(GuardrailValue::Object(bucket)) => matches!(
                    bucket.get(PRE_CALL_EXECUTED_KEY),
                    Some(GuardrailValue::Array(executed)) if executed.contains(&marker)
                ),
                _ => false,
            })
    }
}
