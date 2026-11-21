use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::SystemTime;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::settings::JsonValue;
use betterlite_guardrails::guardrail_base::GuardrailBase;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_records::error::GuardrailRecordError;
use betterlite_guardrails::guardrail_records::invocation::{
    GuardrailHookName, GuardrailInvocation,
};
use betterlite_guardrails::guardrail_records::record::GuardrailRecordContext;
use betterlite_openai::openai_base_config::Object as JsonMap;

use super::bridge::{metadata_buckets, write_buckets};
use super::execute::{GuardrailHook, HookAnswer, HookCall, HookExecution};
use crate::guardrails::callback_caps::errors::ProxyHookError;
use crate::guardrails::callback_caps::traits::PreCallReturn;

// the LiteLLM classes whose own hook carries @log_guardrail_information, one list per hook
const DECORATED_PRE_CALL: [&str; 17] = [
    "Agent365Guardrail",
    "AzureContentSafetyPromptShieldGuardrail",
    "AzureContentSafetyTextModerationGuardrail",
    "CiscoAIDefenseGuardrail",
    "GuardrailsAI",
    "LassoGuardrail",
    "MCPJWTSigner",
    "MCPSecurityGuardrail",
    "MicrosoftPurviewDLPGuardrail",
    "ModelArmorGuardrail",
    "PangeaHandler",
    "PanwPrismaAirsHandler",
    "PillarGuardrail",
    "SemanticGuardrail",
    "ToolPermissionGuardrail",
    "lakeraAI_Moderation",
    "myCustomGuardrail",
];

const DECORATED_MODERATION: [&str; 7] = [
    "AporiaGuardrail",
    "CiscoAIDefenseGuardrail",
    "LassoGuardrail",
    "ModelArmorGuardrail",
    "PillarGuardrail",
    "lakeraAI_Moderation",
    "myCustomGuardrail",
];

const DECORATED_POST_CALL: [&str; 12] = [
    "AporiaGuardrail",
    "CiscoAIDefenseGuardrail",
    "GuardrailsAI",
    "LassoGuardrail",
    "MicrosoftPurviewDLPGuardrail",
    "ModelArmorGuardrail",
    "PangeaHandler",
    "PanwPrismaAirsHandler",
    "PillarGuardrail",
    "SemanticGuardrail",
    "ToolPermissionGuardrail",
    "myCustomGuardrail",
];

#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    #[error("the request data is not guardrail data: {source}")]
    Json {
        #[from]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Record(#[from] GuardrailRecordError),
}

struct OpenRecord {
    started: Result<GuardrailInvocation, RecordError>,
    base: Arc<GuardrailBase>,
    finished_at: SystemTime,
}

// the callback's own hook, inside B3664's invocation record when its LiteLLM class decorates that hook
pub async fn run_own_hook(
    execution: &HookExecution,
    call: HookCall<'_>,
    data: &mut JsonMap,
) -> Result<HookAnswer, ProxyHookError> {
    let Some((hook_name, base)) = decorated_base(execution, &call) else {
        return own_hook(call, data).await;
    };
    let started_at = execution.clock.now();
    let started =
        serde_json::from_value::<BTreeMap<String, GuardrailValue>>(JsonValue::Object(data.clone()))
            .map_err(RecordError::from)
            .and_then(|request_data| {
                GuardrailInvocation::begin(hook_name, &request_data, None, started_at)
                    .map_err(RecordError::from)
            });
    let outcome = own_hook(call, data).await;
    let record = OpenRecord {
        started,
        base,
        finished_at: execution.clock.now(),
    };
    if let Err(failure) = close_record(execution, record, data, &outcome) {
        tracing::warn!("the guardrail hook call was not recorded: {failure}");
    }
    outcome
}

fn decorated_base(
    execution: &HookExecution,
    call: &HookCall<'_>,
) -> Option<(GuardrailHookName, Arc<GuardrailBase>)> {
    let (hook_name, decorated) = match call.hook {
        GuardrailHook::PreCall => (GuardrailHookName::PreCall, DECORATED_PRE_CALL.as_slice()),
        GuardrailHook::DuringCall => (
            GuardrailHookName::Moderation,
            DECORATED_MODERATION.as_slice(),
        ),
        GuardrailHook::PostCall(_) => (
            GuardrailHookName::PostCallSuccess,
            DECORATED_POST_CALL.as_slice(),
        ),
    };
    if !decorated.contains(&call.callback.class_name()) {
        return None;
    }
    execution
        .bases
        .base(call.callback)
        .map(|base| (hook_name, base))
}

fn close_record(
    execution: &HookExecution,
    record: OpenRecord,
    data: &mut JsonMap,
    outcome: &Result<HookAnswer, ProxyHookError>,
) -> Result<(), RecordError> {
    let invocation = record.started?;
    let mut buckets = metadata_buckets(data)?;
    let mut context = GuardrailRecordContext {
        base: &record.base,
        request_data: &mut buckets,
        log_target: None,
        sink: execution.sink.as_ref(),
    };
    match outcome {
        Ok(answer) => {
            let response = answer_value(answer)?;
            invocation.finish_success(&mut context, response.as_ref(), record.finished_at)?;
        }
        Err(error) => {
            let failure = guardrail_failure(error);
            invocation.finish_failure(&mut context, failure, record.finished_at)?;
        }
    }
    write_buckets(data, &buckets)?;
    Ok(())
}

async fn own_hook(call: HookCall<'_>, data: &mut JsonMap) -> Result<HookAnswer, ProxyHookError> {
    let HookCall {
        callback,
        hook,
        context,
    } = call;
    match hook {
        GuardrailHook::PreCall => {
            let Some(user_api_key_dict) = context.user_api_key_dict else {
                return Err(ProxyHookError::Other(format!(
                    "the pre-call hook of {} needs the caller's key",
                    callback.class_name()
                )));
            };
            callback
                .pre_call(user_api_key_dict, data, context.call_type)
                .await
                .map(HookAnswer::PreCall)
        }
        GuardrailHook::DuringCall => {
            callback
                .moderation(data, context.user_api_key_dict, context.call_type)
                .await?;
            Ok(HookAnswer::DuringCall)
        }
        GuardrailHook::PostCall(response) => callback
            .post_call_success(data, context.user_api_key_dict, response)
            .await
            .map(HookAnswer::PostCall),
    }
}

// what the decorated hook returned, which the record summarizes
fn answer_value(answer: &HookAnswer) -> Result<Option<GuardrailValue>, serde_json::Error> {
    let value = match answer {
        HookAnswer::PreCall(PreCallReturn::Data(replacement)) => {
            JsonValue::Object(replacement.clone())
        }
        HookAnswer::PreCall(PreCallReturn::Text(text)) => JsonValue::String(text.clone()),
        HookAnswer::PostCall(Some(response)) => response.clone(),
        HookAnswer::PreCall(PreCallReturn::Unchanged)
        | HookAnswer::DuringCall
        | HookAnswer::PostCall(None) => return Ok(None),
    };
    serde_json::from_value(value).map(Some)
}

// the record reads only the error text and whether it is an intervention
fn guardrail_failure(error: &ProxyHookError) -> GuardrailFailure {
    match error {
        ProxyHookError::Http { status, detail } => GuardrailFailure::Http {
            status: *status,
            detail: detail.clone(),
        },
        ProxyHookError::GuardrailRaised {
            message,
            guardrail_name,
            status,
            blocked_content,
        } => GuardrailFailure::GuardrailRaised {
            guardrail_name: guardrail_name.clone(),
            message: message.clone(),
            wrap_default_message: false,
            status_code: *status,
            blocked_content: *blocked_content,
        },
        ProxyHookError::SensitiveDataRoute(_) | ProxyHookError::ModifyResponse(_) => {
            GuardrailFailure::GuardrailRaised {
                guardrail_name: None,
                message: error.to_string(),
                wrap_default_message: false,
                status_code: 400,
                blocked_content: false,
            }
        }
        ProxyHookError::Proxy(_)
        | ProxyHookError::RejectedRequest { .. }
        | ProxyHookError::Other(_) => GuardrailFailure::Other(error.to_string()),
    }
}
