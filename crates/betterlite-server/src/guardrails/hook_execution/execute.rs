use std::sync::Arc;

use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_config::sections::settings::JsonValue;
use betterlite_core::routes::CallType;
use betterlite_guardrails::guardrail_base::GuardrailBase;
use betterlite_guardrails::guardrail_records::record::GuardrailSpanSink;
use betterlite_guardrails::router_guardrail_selection::error::GuardrailSelectionError;
use betterlite_openai::openai_base_config::Object as JsonMap;

use super::metrics::GuardrailMetrics;
use super::own_hook::run_own_hook;
use super::sensitive_route::SensitiveRouting;
use crate::guardrails::callback_caps::errors::ProxyHookError;
use crate::guardrails::callback_caps::traits::{PreCallReturn, ProxyCallback};
use crate::guardrails::unified_guardrail::apply_record::InvocationClock;
use crate::guardrails::unified_guardrail::recorder::AppliedGuardrailRecorder;

// LiteLLM's hook_type; the post-call hook carries the response it checks
#[derive(Debug, Clone, PartialEq)]
pub enum GuardrailHook {
    PreCall,
    DuringCall,
    PostCall(JsonValue),
}

impl GuardrailHook {
    #[must_use]
    pub const fn hook_type(&self) -> &'static str {
        match self {
            Self::PreCall => "pre_call",
            Self::DuringCall => "during_call",
            Self::PostCall(_) => "post_call",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum HookAnswer {
    PreCall(PreCallReturn),
    DuringCall,
    PostCall(Option<JsonValue>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookPath {
    Unified,
    Own,
}

#[derive(Clone, Copy)]
pub struct CallContext<'call> {
    pub user_api_key_dict: Option<&'call UserAPIKeyAuth>,
    pub call_type: CallType,
}

pub struct HookCall<'call> {
    pub callback: &'call dyn ProxyCallback,
    pub hook: GuardrailHook,
    pub context: CallContext<'call>,
}

// the guardrail goes in as an argument, where LiteLLM writes it into data["guardrail_to_apply"]
pub struct UnifiedCall<'call> {
    pub guardrail: &'call dyn ProxyCallback,
    pub hook: GuardrailHook,
    pub data: &'call mut JsonMap,
    pub context: CallContext<'call>,
}

// B3680's UnifiedGuardrailHooks, at the one hook type _execute_guardrail_hook dispatches on
#[async_trait]
pub trait UnifiedHooks: Send + Sync {
    async fn run(&self, call: UnifiedCall<'_>) -> Result<HookAnswer, ProxyHookError>;
}

#[derive(Debug, thiserror::Error)]
pub enum BalanceError {
    #[error(transparent)]
    Selection(#[from] GuardrailSelectionError),
    #[error("No callback found for guardrail: {guardrail_name}")]
    NoCallback { guardrail_name: String },
}

// the router side of LiteLLM's guardrail load balancing
pub trait GuardrailBalancer: Send + Sync {
    fn should_use_load_balancing(&self, guardrail_name: &str) -> bool;
    fn available_guardrail(
        &self,
        guardrail_name: &str,
    ) -> Result<Arc<dyn ProxyCallback>, BalanceError>;
}

// the CustomGuardrail settings behind a guardrail callback, which the invocation record and the error context read
pub trait GuardrailBases: Send + Sync {
    fn base(&self, callback: &dyn ProxyCallback) -> Option<Arc<GuardrailBase>>;
}

pub struct HookExecution {
    pub unified: Arc<dyn UnifiedHooks>,
    pub balancer: Arc<dyn GuardrailBalancer>,
    pub bases: Arc<dyn GuardrailBases>,
    pub recorder: Arc<dyn AppliedGuardrailRecorder + Send>,
    // None when no Prometheus logger is configured
    pub metrics: Option<Arc<dyn GuardrailMetrics>>,
    // None when the sensitive_data_routing proxy hook is not registered
    pub sensitive_routing: Option<Arc<dyn SensitiveRouting>>,
    pub sink: Arc<dyn GuardrailSpanSink + Send + Sync>,
    pub clock: Arc<dyn InvocationClock>,
}

#[must_use]
pub fn hook_path(
    callback: &dyn ProxyCallback,
    hook: &GuardrailHook,
    context: CallContext<'_>,
) -> HookPath {
    let native_during_call = matches!(hook, GuardrailHook::DuringCall)
        && callback
            .guardrail()
            .is_some_and(|guardrail| guardrail.use_native_during_call_hook());
    // _run_during_call_guardrail takes the unified path only when it holds the caller's key
    let unkeyed_during_call =
        matches!(hook, GuardrailHook::DuringCall) && context.user_api_key_dict.is_none();
    if callback.overrides().apply_guardrail
        && !callback.use_native_lifecycle_hooks()
        && !native_during_call
        && !unkeyed_during_call
    {
        HookPath::Unified
    } else {
        HookPath::Own
    }
}

impl HookExecution {
    pub async fn execute_guardrail_hook(
        &self,
        call: HookCall<'_>,
        data: &mut JsonMap,
    ) -> Result<HookAnswer, ProxyHookError> {
        match hook_path(call.callback, &call.hook, call.context) {
            HookPath::Own => run_own_hook(self, call, data).await,
            HookPath::Unified => {
                self.unified
                    .run(UnifiedCall {
                        guardrail: call.callback,
                        hook: call.hook,
                        data,
                        context: call.context,
                    })
                    .await
            }
        }
    }

    pub async fn execute_with_load_balancing(
        &self,
        guardrail_name: &str,
        call: HookCall<'_>,
        data: &mut JsonMap,
    ) -> Result<HookAnswer, ProxyHookError> {
        let selected = self
            .balancer
            .available_guardrail(guardrail_name)
            .map_err(|error| ProxyHookError::Other(error.to_string()))?;
        let picked = HookCall {
            callback: selected.as_ref(),
            hook: call.hook,
            context: call.context,
        };
        self.execute_guardrail_hook(picked, data).await
    }
}

pub fn process_pre_call_response(
    response: PreCallReturn,
    data: &mut JsonMap,
    call_type: CallType,
) -> Result<(), ProxyHookError> {
    match response {
        PreCallReturn::Unchanged => Ok(()),
        PreCallReturn::Data(replacement) => {
            *data = replacement;
            Ok(())
        }
        PreCallReturn::Text(message) => {
            if matches!(call_type, CallType::Completion | CallType::TextCompletion) {
                let model = data.get("model").and_then(JsonValue::as_str);
                return Err(ProxyHookError::RejectedRequest {
                    message,
                    model: model.unwrap_or_default().to_owned(),
                });
            }
            let detail = JsonMap::from_iter([("error".to_owned(), JsonValue::String(message))]);
            Err(ProxyHookError::Http {
                status: 400,
                detail: JsonValue::Object(detail).to_string(),
            })
        }
    }
}
