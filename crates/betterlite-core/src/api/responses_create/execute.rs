use super::bridge::{Created, StampedResponse};
use super::call::ResponsesCreateCall;
use super::convert::{decision_json, json_value, reencode};
use super::deps::{ConfigRequest, CreateCapabilities, CreateConfigFactory, ResponsesCreateDeps};
use super::error::{CreateError, CreateParamError};
use super::interception::{CreateInterception, FileSearchExtras, Interception, dispatch_args};
use super::native::{bridge, native};
use super::preparation::{self, Prepared};
use crate::api::responses_agentic_execution::executor::ContinuationExecutor;
use crate::api::responses_create_http::service::{ClientSource, LogContext};
use crate::api::responses_hook_policy::callbacks::DeploymentHookDispatcher;
use crate::api::responses_parameters::requested::requested_optional_params;
use crate::api::responses_provider_selection::ParameterMap;
use crate::api::responses_provider_selection::file_ids::apply_managed_file_id_mapping;
use crate::api::responses_provider_selection::provider_config::{
    bridges_to_chat_completions, encrypted_task_support_failure, resolve_provider_config,
};
use betterlite_config::env_integer::timeout::RequestTimeout;
use betterlite_context::non_default_params::Params;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::HiddenParams;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_prompts::file_ids::mapping::FileIdMapping;
use betterlite_prompts::file_ids::responses::ResponsesInput as PromptInput;
use betterlite_prompts::file_ids::tools::ResponsesTool as PromptTool;
use indexmap::IndexMap;
// responses()'s whole synchronous path, in the order the source takes each step
pub async fn create<S, F, D, E, Context>(
    deps: &ResponsesCreateDeps<'_, S, F, D, E, Context>,
    call: &ResponsesCreateCall,
    context: Context,
    log: &mut LogContext<'_, '_>,
) -> Result<Created<Context>, CreateError>
where
    S: ClientSource,
    F: CreateConfigFactory,
    D: DeploymentHookDispatcher + ?Sized,
    E: ContinuationExecutor + ?Sized,
    Context: Send,
{
    let request_timeout = RequestTimeout::resolve(deps.environment)?;
    let mut prep = preparation::begin(call, deps.resolver, deps.lookup, deps.prompts)?;
    preparation::affinity(&mut prep, call)?;
    if let Some(text) = mock_text(call) {
        let response = mock_responses_api_response(Some(text))?;
        return Ok(Created::Responses(Box::new(StampedResponse {
            response,
            hidden: HiddenParams::new(),
        })));
    }
    let resolved = preparation::resolve(&mut prep, call, deps.resolver)?;
    let tools = mapped_tools(&mut prep, deps.file_mapping)?;
    let context = if prep.skip_mcp || !deps.interception.mcp_requested(tools.as_deref()) {
        context
    } else {
        let args = dispatch_args(
            &prep,
            prep.call.reasoning.as_ref(),
            tools.as_deref(),
            &prep.kwargs,
        );
        match deps.interception.mcp(&args, context).await? {
            Interception::Handled(created) => return Ok(*created),
            Interception::Continue(next) => next,
        }
    };
    let model_info = json_value(&resolved.deployment_model_info);
    let config = resolve_provider_config(
        deps.lookup,
        &resolved.model,
        &resolved.provider,
        &model_info,
        resolved.api_base.as_deref(),
    );
    let request = ConfigRequest {
        model: &resolved.model,
        provider: &resolved.provider,
        api_base: resolved.api_base.as_deref(),
        model_info: &model_info,
    };
    let provider_config = deps.configs.create_config(&request);
    if prep.require_encrypted
        && let Some(failure) = encrypted_task_support_failure(config.as_ref(), prep.use_chat)
    {
        return Err(CreateError::Compatibility(failure));
    }
    let (requested, reasoning) = requested_params(&prep, deps.reasoning_auto_summary)?;
    let native_search = provider_config
        .as_ref()
        .is_some_and(CreateCapabilities::supports_native_file_search);
    let emulated = has_file_search_tool(tools.as_deref())
        && (config.is_none() || prep.use_chat || !native_search);
    let context = if emulated {
        let mut kwargs = prep.kwargs.clone();
        kwargs.shift_remove("litellm_call_id");
        let mapped_reasoning =
            (!matches!(reasoning, JsonValue::Null)).then(|| decision_json(&reasoning));
        let args = dispatch_args(&prep, mapped_reasoning.as_ref(), tools.as_deref(), &kwargs);
        let extras = FileSearchExtras {
            service_tier: prep.call.service_tier.as_deref(),
            safety_identifier: prep.call.safety_identifier.as_deref(),
            text_format: prep.call.text_format.as_ref(),
            allowed_openai_params: prep.call.allowed_openai_params.as_deref(),
            use_chat_completions_api: prep.use_chat,
        };
        match deps
            .interception
            .file_search(&args, &extras, context)
            .await?
        {
            Interception::Handled(created) => return Ok(*created),
            Interception::Continue(next) => next,
        }
    } else {
        context
    };
    if bridges_to_chat_completions(config.as_ref(), prep.use_chat) {
        return bridge(
            deps,
            &prep,
            call,
            config.as_ref(),
            &requested,
            request_timeout,
            context,
        )
        .await;
    }
    let provider_config = provider_config.ok_or_else(|| CreateError::Unsupported {
        provider: resolved.provider.clone(),
    })?;
    native(
        deps,
        &prep,
        call,
        &resolved,
        &provider_config,
        requested,
        request_timeout,
        log,
    )
    .await
}
// litellm_params.mock_response is the call's own truthy string; an empty or non-string value skips the shortcut
fn mock_text(call: &ResponsesCreateCall) -> Option<&str> {
    match call.kwargs.get("mock_response") {
        Some(JsonValue::Text(text)) => Some(text.as_str()).filter(|text| !text.is_empty()),
        Some(
            JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::Array(_)
            | JsonValue::Object(_),
        )
        | None => None,
    }
}
// _apply_managed_file_id_mapping rewrites the input and tools before any dispatch sees them
fn mapped_tools(
    prep: &mut Prepared<'_>,
    mapping: Option<&FileIdMapping>,
) -> Result<Option<Vec<JsonValue>>, CreateError> {
    let input: PromptInput = reencode(&prep.input)?;
    let tools = prep
        .call
        .tools
        .as_ref()
        .map(|tools| {
            tools
                .iter()
                .map(|tool| reencode::<_, PromptTool>(&tool.0))
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?;
    let model_info = prep
        .kwargs
        .get("model_info")
        .filter(|value| **value != JsonValue::Null);
    let outcome = apply_managed_file_id_mapping(
        &input,
        tools.as_deref(),
        model_info,
        mapping,
        &mut prep.local_vars,
    )?;
    prep.input = reencode(&outcome.input)?;
    outcome
        .tools
        .as_ref()
        .map(|tools| {
            tools
                .iter()
                .map(|tool| reencode::<_, JsonValue>(tool))
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()
        .map_err(CreateError::from)
}
// local_vars.update(kwargs), the mapped reasoning, then get_requested_response_api_optional_param
fn requested_params(
    prep: &Prepared<'_>,
    auto_summary: bool,
) -> Result<(Params, JsonValue), CreateError> {
    let mut locals: Params = prep
        .local_vars
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .chain(
            prep.kwargs
                .iter()
                .map(|(name, value)| (name.clone(), value.clone())),
        )
        .collect();
    locals.insert(
        String::from("kwargs"),
        JsonValue::Object(prep.kwargs.clone()),
    );
    let reasoning = request_reasoning(&locals, auto_summary)?;
    locals.insert(String::from("reasoning"), reasoning.clone());
    locals.remove("reasoning_effort");
    let requested = requested_optional_params(&mut locals)
        .map_err(CreateParamError::from)
        .map_err(CreateError::from)?;
    Ok((requested, reasoning))
}
// _map_reasoning_effort: an object is the Reasoning itself; a string becomes one, with the auto summary attached
fn request_reasoning(locals: &Params, auto_summary: bool) -> Result<JsonValue, CreateError> {
    if let Some(current) = locals
        .get("reasoning")
        .filter(|value| **value != JsonValue::Null)
    {
        return Ok(current.clone());
    }
    let Some(effort) = locals
        .get("reasoning_effort")
        .filter(|value| **value != JsonValue::Null)
    else {
        return Ok(JsonValue::Null);
    };
    match effort {
        JsonValue::Object(_) => Ok(effort.clone()),
        JsonValue::Text(text) => {
            let mut fields = IndexMap::new();
            fields.insert(String::from("effort"), JsonValue::Text(text.clone()));
            if auto_summary {
                fields.insert(
                    String::from("summary"),
                    JsonValue::Text(String::from("detailed")),
                );
            }
            Ok(JsonValue::Object(fields))
        }
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Array(_) => {
            Err(CreateError::Field {
                field: "reasoning_effort",
                expected: "a string or an object",
            })
        }
    }
}
pub const DEFAULT_MOCK_TEXT: &str = "In a peaceful grove beneath a silver moon, a unicorn named Lumina discovered a hidden pool that reflected the stars. As she dipped her horn into the water, the pool began to shimmer, revealing a pathway to a magical realm of endless night skies. Filled with wonder, Lumina whispered a wish for all who dream to find their own hidden magic, and as she glanced back, her hoofprints sparkled like stardust.";
// mock_responses_api_response's literal body, with the one caller-chosen field left as a placeholder
const MOCK_RESPONSE: &str = r#"{"id":"resp_67ccd2bed1ec8190b14f964abc0542670bb6a6b452d3795b","object":"response","created_at":1741476542,"status":"completed","error":null,"incomplete_details":null,"instructions":null,"max_output_tokens":null,"model":"gpt-4.1-2025-04-14","output":[{"type":"message","id":"msg_67ccd2bf17f0819081ff3bb2cf6508e60bb6a6b452d3795b","status":"completed","role":"assistant","content":[{"type":"output_text","text":$TEXT,"annotations":[]}]}],"parallel_tool_calls":true,"previous_response_id":null,"reasoning":{"effort":null,"summary":null},"store":true,"temperature":1.0,"text":{"format":{"type":"text"}},"tool_choice":"auto","tools":[],"top_p":1.0,"truncation":"disabled","usage":{"input_tokens":36,"input_tokens_details":{"cached_tokens":0},"output_tokens":87,"output_tokens_details":{},"total_tokens":123},"user":null,"metadata":{}}"#;
pub fn mock_responses_api_response(
    mock_response: Option<&str>,
) -> Result<ResponsesApiResponse, serde_json::Error> {
    serde_json::from_str(&MOCK_RESPONSE.replace(
        "$TEXT",
        &serde_json::to_string(mock_response.unwrap_or(DEFAULT_MOCK_TEXT))?,
    ))
}
// _has_file_search_tool: any tool entry that is an object with type file_search
#[must_use]
pub fn has_file_search_tool(tools: Option<&[JsonValue]>) -> bool {
    tools.is_some_and(|present| {
        present.iter().any(|tool| {
            matches!(tool, JsonValue::Object(fields) if matches!(fields.get("type"), Some(JsonValue::Text(kind)) if kind == "file_search"))
        })
    })
}
