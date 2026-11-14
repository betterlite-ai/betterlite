use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use uuid::Uuid;

use crate::guardrail_base::GuardrailBase;

// LiteLLM's PRE_CALL_EXECUTED_GUARDRAILS_KEY; the name limit drops GUARDRAILS
pub const PRE_CALL_EXECUTED_KEY: &str = "_pre_call_executed_guardrails";

// The per-process secret that tags every pre-call marker; startup makes one and passes it to every call
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreCallToken(pub String);

// LiteLLM's secrets.token_hex(16): 32 lowercase hex characters
impl From<Uuid> for PreCallToken {
    fn from(id: Uuid) -> Self {
        Self(id.simple().to_string())
    }
}

impl GuardrailBase {
    // LiteLLM's _pre_call_marker; an empty name is used as given
    #[must_use]
    pub fn pre_call_marker(&self, token: &PreCallToken) -> Option<String> {
        self.settings
            .guardrail_name
            .as_ref()
            .map(|name| format!("{}:{name}", token.0))
    }

    // Records that this guardrail's pre-call hook ran, in the first of metadata and litellm_metadata that is an object
    pub fn mark_pre_call_hook_ran(
        &self,
        token: &PreCallToken,
        data: &mut BTreeMap<String, GuardrailValue>,
    ) {
        let Some(text) = self.pre_call_marker(token) else {
            return;
        };
        let marker = GuardrailValue::Text(text);
        for location in ["metadata", "litellm_metadata"] {
            let Some(GuardrailValue::Object(bucket)) = data.get_mut(location) else {
                continue;
            };
            match bucket.get_mut(PRE_CALL_EXECUTED_KEY) {
                Some(GuardrailValue::Array(executed)) => {
                    if !executed.contains(&marker) {
                        executed.push(marker);
                    }
                }
                _ => {
                    bucket.insert(
                        PRE_CALL_EXECUTED_KEY.to_owned(),
                        GuardrailValue::Array(vec![marker]),
                    );
                }
            }
            return;
        }
        data.insert(
            "metadata".to_owned(),
            GuardrailValue::Object(BTreeMap::from([(
                PRE_CALL_EXECUTED_KEY.to_owned(),
                GuardrailValue::Array(vec![marker]),
            )])),
        );
    }

    // True when the marker is in the list of either bucket
    #[must_use]
    pub fn pre_call_hook_already_ran(
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
