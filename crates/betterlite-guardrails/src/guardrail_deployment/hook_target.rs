use std::collections::BTreeMap;
use std::future::Future;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::routes::CallType;

use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::failure::GuardrailFailure;

type Data = BTreeMap<String, GuardrailValue>;

// Which code runs the hook: the guardrail's own, or the unified guardrail that runs this guardrail
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookTarget {
    Own,
    Unified,
}

// The UserAPIKeyAuth fields the hooks read back from the request metadata
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StampedKeyFields {
    pub user_id: Option<String>,
    pub team_id: Option<String>,
    pub end_user_id: Option<String>,
    pub api_key: Option<String>,
    pub request_route: Option<String>,
}

// What async_pre_call_hook gives back: None, a dict, a string or a returned exception; the failure is boxed to keep the variants close in size
#[derive(Debug, Clone, PartialEq)]
pub enum PreCallOutcome {
    Unchanged,
    Data(Data),
    Text(String),
    Failure(Box<GuardrailFailure>),
}

// The caller implements this for one guardrail; Own is its own hook, Unified the unified guardrail running it
pub trait DeploymentHookTarget: Sync {
    fn pre_call(
        &self,
        target: HookTarget,
        key: &StampedKeyFields,
        data: &mut Data,
        call_type: CallType,
    ) -> impl Future<Output = Result<PreCallOutcome, GuardrailFailure>> + Send;

    fn post_call_success(
        &self,
        target: HookTarget,
        key: &StampedKeyFields,
        data: &mut Data,
        response: GuardrailValue,
    ) -> impl Future<Output = Result<Option<GuardrailValue>, GuardrailFailure>> + Send;
}

// LiteLLM's get_metadata_variable_name_from_kwargs, then the value under that name
#[must_use]
pub fn metadata_bucket(data: &Data) -> Option<&GuardrailValue> {
    if data.contains_key("litellm_metadata") {
        data.get("litellm_metadata")
    } else {
        data.get("metadata")
    }
}

// LiteLLM's _user_api_key_auth_from_request
#[must_use]
pub fn stamped_key_fields(data: &Data) -> StampedKeyFields {
    let stamped = match metadata_bucket(data) {
        Some(GuardrailValue::Object(fields)) => Some(fields),
        _ => None,
    };
    let text = |field: &str| match stamped.and_then(|fields| fields.get(field)) {
        Some(GuardrailValue::Text(value)) => Some(value.clone()),
        _ => None,
    };
    StampedKeyFields {
        user_id: text("user_api_key_user_id"),
        team_id: text("user_api_key_team_id"),
        end_user_id: text("user_api_key_end_user_id"),
        api_key: text("user_api_key_hash"),
        request_route: text("user_api_key_request_route"),
    }
}

impl GuardrailBase {
    // LiteLLM's _deployment_hook_target; it raises ImportError without the proxy extras, which betterlite always has
    #[must_use]
    pub const fn hook_target(&self) -> HookTarget {
        let switches = &self.settings.switches;
        if !switches.uses_apply_guardrail_interface || switches.use_native_lifecycle_hooks {
            HookTarget::Own
        } else {
            HookTarget::Unified
        }
    }
}
