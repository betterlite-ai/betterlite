use super::bridge::{Created, StampedResponse};
use super::call::ResponsesCreateCall;
use super::deps::{ConfigRequest, CreateCapabilities, CreateConfigFactory, ResponsesCreateDeps};
use super::error::CreateError;
use super::interception::{FileSearchArgs, Interception, dispatch_args};
use super::native::{NativeCall, bridge, native};
use super::preparation::{self, Prepared, Resolved};
use crate::api::response_id_traversal::response_id::transcode;
use crate::api::responses_agentic_execution::executor::ContinuationExecutor;
use crate::api::responses_compact_http::service::ClientSource;
use crate::api::responses_create_http::service::{CreateTransport, LogContext};
use crate::api::responses_hook_policy::callbacks::DeploymentHookDispatcher;
use crate::api::responses_json::{decision_json, json_value};
use crate::api::responses_parameters::requested::requested_optional_params;
use crate::api::responses_provider_selection::file_ids::apply_managed_file_id_mapping;
use crate::api::responses_provider_selection::provider_config::{
    ResponsesApiConfig, bridges_to_chat_completions, encrypted_task_support_failure,
    resolve_provider_config,
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
pub async fn create<S, F, D, E, Context>(
    deps: &ResponsesCreateDeps<'_, S, F, D, E, Context>,
    call: &ResponsesCreateCall,
    mut context: Context,
    log: &mut LogContext<'_, '_>,
) -> Result<Created<Context>, CreateError>
where
    S: ClientSource<Client: CreateTransport>,
    F: CreateConfigFactory,
    D: DeploymentHookDispatcher + ?Sized,
    E: ContinuationExecutor + ?Sized,
    Context: Send,
{
    let timeout = RequestTimeout::resolve(deps.environment)?;
    let mut prep = preparation::begin(call, deps.resolver, deps.lookup, deps.prompts)?;
    preparation::affinity(&mut prep, call)?;
    if let Some(created) = mock_created(&prep)? {
        return Ok(created);
    }
    let resolved = preparation::resolve(&mut prep, call, deps.resolver)?;
    let tools = mapped_tools(&mut prep, deps.file_mapping)?;
    match deps.mcp_step(&prep, tools.as_deref(), context).await? {
        Interception::Handled(created) => return Ok(*created),
        Interception::Continue(next) => context = next,
    }
    let setup = deps.setup(&prep, &resolved, tools.as_deref())?;
    if setup.emulated {
        let args = (tools.as_deref(), &setup.reasoning);
        match deps.file_search_step(&prep, args, context).await? {
            Interception::Handled(created) => return Ok(*created),
            Interception::Continue(next) => context = next,
        }
    }
    let supported = setup.supported.as_ref();
    if bridges_to_chat_completions(supported, prep.use_chat.0) {
        return bridge(deps, &prep, supported, &setup.requested, context).await;
    }
    let Some(provider) = setup.provider else {
        return Err(CreateError::Unsupported {
            provider: resolved.provider.clone(),
        });
    };
    let native_call = NativeCall {
        prep: &prep,
        resolved: &resolved,
        provider_config: &provider,
        requested: setup.requested,
        timeout,
    };
    native(deps, native_call, log).await
}
struct Setup<Provider> {
    supported: Option<ResponsesApiConfig>,
    provider: Option<Provider>,
    requested: Params,
    reasoning: JsonValue,
    emulated: bool,
}
fn mock_created<Context>(prep: &Prepared<'_>) -> Result<Option<Created<Context>>, CreateError> {
    let Some(JsonValue::Text(text)) = prep.kwargs.get("mock_response") else {
        return Ok(None);
    };
    if text.is_empty() {
        return Ok(None);
    }
    let response = mock_responses_api_response(Some(text.as_str()))?;
    Ok(Some(Created::Responses(Box::new(StampedResponse {
        response,
        hidden: HiddenParams::new(),
    }))))
}
impl<S, F, D, E, Context> ResponsesCreateDeps<'_, S, F, D, E, Context>
where
    S: ClientSource<Client: CreateTransport>,
    F: CreateConfigFactory,
    D: DeploymentHookDispatcher + ?Sized,
    E: ContinuationExecutor + ?Sized,
    Context: Send,
{
    async fn mcp_step(
        &self,
        prep: &Prepared<'_>,
        tools: Option<&[JsonValue]>,
        context: Context,
    ) -> Result<Interception<Context>, CreateError> {
        if prep.skip_mcp.0 || !self.interception.mcp_requested(tools) {
            return Ok(Interception::Continue(context));
        }
        let args = dispatch_args(prep, prep.call.reasoning.as_ref(), tools, &prep.kwargs);
        self.interception
            .mcp(&args, context)
            .await
            .map_err(CreateError::from)
    }
    async fn file_search_step(
        &self,
        prep: &Prepared<'_>,
        args: (Option<&[JsonValue]>, &JsonValue),
        context: Context,
    ) -> Result<Interception<Context>, CreateError> {
        let (tools, reasoning) = args;
        let mut kwargs = prep.kwargs.clone();
        kwargs.shift_remove("litellm_call_id");
        let mapped = (!matches!(reasoning, JsonValue::Null)).then(|| decision_json(reasoning));
        let call = dispatch_args(prep, mapped.as_ref(), tools, &kwargs);
        let search = FileSearchArgs {
            call,
            service_tier: prep.call.service_tier.as_deref(),
            safety_identifier: prep.call.safety_identifier.as_deref(),
            text_format: prep.call.text_format.as_ref(),
            allowed_openai_params: prep.call.allowed_openai_params.as_deref(),
            use_chat_completions_api: prep.use_chat.0,
        };
        self.interception
            .file_search(&search, context)
            .await
            .map_err(CreateError::from)
    }
    fn setup(
        &self,
        prep: &Prepared<'_>,
        resolved: &Resolved,
        tools: Option<&[JsonValue]>,
    ) -> Result<Setup<F::Provider>, CreateError> {
        let model_info = json_value(&resolved.deployment_model_info);
        let supported = resolve_provider_config(
            self.lookup,
            &resolved.model,
            &resolved.provider,
            &model_info,
            resolved.api_base.as_deref(),
        );
        let provider = self.configs.create_config(&ConfigRequest {
            model: &resolved.model,
            provider: &resolved.provider,
            api_base: resolved.api_base.as_deref(),
            model_info: &model_info,
        });
        if prep.require_encrypted.0
            && let Some(failure) =
                encrypted_task_support_failure(supported.as_ref(), prep.use_chat.0)
        {
            return Err(CreateError::Compatibility(failure));
        }
        let (requested, reasoning) = requested_params(prep, self.reasoning_auto_summary)?;
        let native_search = provider
            .as_ref()
            .is_some_and(CreateCapabilities::supports_native_file_search);
        let emulated = has_file_search_tool(tools)
            && (supported.is_none() || prep.use_chat.0 || !native_search);
        Ok(Setup {
            supported,
            provider,
            requested,
            reasoning,
            emulated,
        })
    }
}
fn mapped_tools(
    prep: &mut Prepared<'_>,
    mapping: Option<&FileIdMapping>,
) -> Result<Option<Vec<JsonValue>>, CreateError> {
    let input: PromptInput = transcode(&prep.input)?;
    let tools: Option<Vec<PromptTool>> = prep
        .call
        .tools
        .as_ref()
        .map(|tools| tools.iter().map(|tool| transcode(&tool.0)).collect())
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
    prep.input = transcode(&outcome.input)?;
    outcome
        .tools
        .as_ref()
        .map(|entries| entries.iter().map(transcode).collect())
        .transpose()
        .map_err(CreateError::from)
}
fn requested_params(
    prep: &Prepared<'_>,
    auto_summary: bool,
) -> Result<(Params, JsonValue), CreateError> {
    let mut locals: Params = prep.local_vars.clone().into_iter().collect();
    locals.extend(prep.kwargs.clone());
    let kwargs = JsonValue::Object(prep.kwargs.clone());
    locals.insert(String::from("kwargs"), kwargs);
    let reasoning = request_reasoning(&locals, auto_summary)?;
    locals.insert(String::from("reasoning"), reasoning.clone());
    locals.remove("reasoning_effort");
    let requested = requested_optional_params(&mut locals)?;
    Ok((requested, reasoning))
}
fn request_reasoning(locals: &Params, auto_summary: bool) -> Result<JsonValue, CreateError> {
    if let Some(current) = locals
        .get("reasoning")
        .filter(|value| **value != JsonValue::Null)
    {
        return Ok(current.clone());
    }
    let given = locals.get("reasoning_effort");
    let Some(effort) = given.filter(|value| **value != JsonValue::Null) else {
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
const MOCK_RESPONSE: &str = r#"{"id":"resp_67ccd2bed1ec8190b14f964abc0542670bb6a6b452d3795b","object":"response","created_at":1741476542,"status":"completed","error":null,"incomplete_details":null,"instructions":null,"max_output_tokens":null,"model":"gpt-4.1-2025-04-14","output":[{"type":"message","id":"msg_67ccd2bf17f0819081ff3bb2cf6508e60bb6a6b452d3795b","status":"completed","role":"assistant","content":[{"type":"output_text","text":$TEXT,"annotations":[]}]}],"parallel_tool_calls":true,"previous_response_id":null,"reasoning":{"effort":null,"summary":null},"store":true,"temperature":1.0,"text":{"format":{"type":"text"}},"tool_choice":"auto","tools":[],"top_p":1.0,"truncation":"disabled","usage":{"input_tokens":36,"input_tokens_details":{"cached_tokens":0},"output_tokens":87,"output_tokens_details":{},"total_tokens":123},"user":null,"metadata":{}}"#;
pub fn mock_responses_api_response(
    mock_response: Option<&str>,
) -> Result<ResponsesApiResponse, serde_json::Error> {
    serde_json::from_str(&MOCK_RESPONSE.replace(
        "$TEXT",
        &serde_json::to_string(mock_response.unwrap_or(DEFAULT_MOCK_TEXT))?,
    ))
}
#[must_use]
pub fn has_file_search_tool(tools: Option<&[JsonValue]>) -> bool {
    tools.is_some_and(|present| {
        present.iter().any(|tool| {
            matches!(tool, JsonValue::Object(fields) if matches!(fields.get("type"), Some(JsonValue::Text(kind)) if kind == "file_search"))
        })
    })
}
