use std::fmt::Display;

use betterlite_bedrock::converse_request::{CommonRequest, ConverseRequestBody};
use betterlite_bedrock::nova_chat::direct::{
    NovaChatArgs, NovaChatConfig, NovaEndpointRequest, SUPPORTED_OPENAI_PARAMS, resolve_endpoint,
    transform_response,
};
use betterlite_bedrock::nova_chat::invoke::{
    InlineRequest, inline_cache_points, transform_converse, without_tool_config_points,
};
use betterlite_logging::logging_pre_post::args::RawBody;
use betterlite_models::decisions::Json as MetadataJson;
use betterlite_openai::compatible_wire::{LogInput, ReplyOptions};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_oracle_runner::helpers::{HelperCall, invoke};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{from_value, json};
use uuid::Uuid;

use super::input::{
    DefaultsInput, DelegationInput, EndpointInput, InlineInput, Input, ResponseModelInput,
    TransformInput,
};
use super::logs::logging_fixture;
use super::signing::secrets;

pub(super) fn helper_error<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: String::from("NovaChat"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::InlineCachePoints(case) => inline_points(case),
        Input::FilterAllowedFields(case) | Input::ToolConfigInjection(case) => pipeline(case),
        Input::Endpoint(case) => endpoint(case),
        Input::ConverseDelegation(case) => delegation(case),
        Input::Defaults(case) => defaults(case),
        Input::ResponseModel(case) => response_model(case),
    }
}

fn inline_points(case: &InlineInput) -> HelperResult {
    let body = inline_cache_points(&InlineRequest {
        system: &case.system,
        messages: &case.messages,
    })
    .map_err(helper_error)?;
    Ok(json!({"system": body.system, "messages": body.messages}))
}

// The Converse body is built by the pinned B604 transformation through its own adapter, then the Nova post-processing runs on it.
fn pipeline(case: &TransformInput) -> HelperResult {
    let filtered = without_tool_config_points(&case.optional_params);
    let request = json!({
        "model": case.model,
        "messages": case.messages,
        "model_cost": case.model_cost,
        "optional_params": filtered,
        "litellm_params": case.litellm_params,
        "global_drop": false,
        "modify_params": false,
        "neutralize_orphaned_blocks": false,
        "request_metadata_fields": null,
    });
    let output = invoke(
        &[crate::GROUP],
        &HelperCall {
            operation: String::from("bedrock_converse_request"),
            input: request,
        },
    );
    if let Some(error) = output.error {
        return Err(Box::new(error));
    }
    let value = output
        .value
        .ok_or_else(|| helper_error("the Converse request returned no body"))?;
    let body = value
        .get("body")
        .ok_or_else(|| helper_error("the Converse request returned no body member"))?;
    let view: ConverseBodyView = from_value(body.clone()).map_err(helper_error)?;
    let body = transform_converse(&view.into_body()).map_err(helper_error)?;
    Ok(json!({"body": body}))
}

// Only the members Nova carries survive into the invoke body; every other Converse member is dropped unread.
#[derive(Deserialize)]
struct ConverseBodyView {
    #[serde(default)]
    messages: Vec<Json>,
    #[serde(rename = "inferenceConfig", default)]
    inference_config: Object,
    #[serde(default)]
    system: Vec<Json>,
    #[serde(rename = "toolConfig", default)]
    tool_config: Option<Json>,
    #[serde(rename = "guardrailConfig", default)]
    guardrail_config: Option<Json>,
}

impl ConverseBodyView {
    fn into_body(self) -> ConverseRequestBody {
        ConverseRequestBody {
            messages: self.messages,
            common: CommonRequest {
                inference_config: self.inference_config,
                additional_fields: None,
                response_paths: None,
                system: self.system,
                guardrail_config: self.guardrail_config,
                performance_config: None,
                service_tier: None,
                tool_config: self.tool_config,
                request_metadata: None,
                output_config: None,
            },
        }
    }
}

fn endpoint(case: &EndpointInput) -> HelperResult {
    let api_key = case.api_key.as_deref().map(SecretString::from);
    let global_key = case.global_api_key.as_deref().map(SecretString::from);
    let litellm_key = case.litellm_api_key.as_deref().map(SecretString::from);
    let request = NovaEndpointRequest {
        api_base: case.api_base.as_deref(),
        api_key: api_key.as_ref(),
        global_api_key: global_key.as_ref(),
        litellm_api_key: litellm_key.as_ref(),
    };
    let resolved =
        resolve_endpoint(request, &case.environment, &secrets()).map_err(helper_error)?;
    Ok(json!({
        "api_base": resolved.api_base,
        "api_key": resolved.api_key.map(|key| key.expose_secret().to_owned()),
    }))
}

fn converse(operation: &str, input: Json) -> Result<Json, Box<HelperError>> {
    let output = invoke(
        &[crate::GROUP],
        &HelperCall {
            operation: operation.to_owned(),
            input,
        },
    );
    if let Some(error) = output.error {
        return Err(Box::new(error));
    }
    output
        .value
        .ok_or_else(|| helper_error("the Converse adapter returned nothing"))
}

// The Nova invoke config has no option or mapping code of its own: both come from the Converse adapters.
fn delegation(case: &DelegationInput) -> HelperResult {
    let supported = converse(
        "bedrock_converse_options",
        json!({
            "function": "params",
            "model_cost": case.model_cost,
            "models": [case.model],
        }),
    )?;
    let mapped = converse(
        "bedrock_converse_params",
        json!({
            "function": "map_params",
            "model": case.model,
            "model_cost": case.model_cost,
            "environment": case.environment,
            "non_default_params": case.non_default_params,
            "optional_params": case.optional_params,
            "global_drop": false,
            "call_drop": false,
        }),
    )?;
    Ok(json!({
        "supported": supported.pointer("/results/0"),
        "params": mapped.pointer("/params"),
    }))
}

fn defaults(case: &DefaultsInput) -> HelperResult {
    let arguments: NovaChatArgs =
        from_value(serde_json::to_value(&case.arguments).map_err(helper_error)?)
            .map_err(helper_error)?;
    let config = NovaChatConfig {
        args: arguments,
        metadata: None,
    };
    let encoded = config.config().map_err(helper_error)?;
    Ok(json!({"config": encoded, "supported": SUPPORTED_OPENAI_PARAMS}))
}

fn response_model(case: &ResponseModelInput) -> HelperResult {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(helper_error)?;
    let (mut state, env) = logging_fixture(&redactor)?;
    let options = ReplyOptions {
        json_mode: false,
        provider: None,
        base_model: None,
        identifier: || Uuid::nil(),
        clock: || 1_700_000_000_i64,
    };
    let input = LogInput {
        messages: MetadataJson::List(Vec::new()),
        data: RawBody::default(),
    };
    let reply = transform_response(
        &case.model,
        case.body.as_bytes(),
        &options,
        &mut state,
        &env,
        input,
    )
    .map_err(helper_error)?;
    Ok(json!({"model": reply.response.model}))
}
