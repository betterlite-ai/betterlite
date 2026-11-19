use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::edenai_messages::{
    EnvironmentInput, ReplyInput, descriptor, transform_response as messages_transform_response,
    validate_environment as messages_validate_environment,
};
use betterlite_core::json_messages_policy::{JsonMessagesPolicy, MessagesCredentials};
use betterlite_logging::logging_pre_post::args::RawBody;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::{HiddenParams, Json as HiddenJson};
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::chat_content::{Mode, RefuseDownloads};
use betterlite_openai::compatible_wire::{LogInput, ReplyOptions};
use betterlite_openai::edenai_chat::{
    CatalogReply, ReasoningLookup, ReplyRequest, TransformInput, chunk_parser, get_models,
    models_url, supported_params, transform_request, transform_response as chat_transform_response,
};
use betterlite_openai::edenai_common::{
    EdenAiEnv, RESPONSE_COST_HEADER, reported_cost, reported_cost_from_bytes,
    reported_cost_from_headers, require_api_key, resolve_api_base, resolve_api_key,
};
use betterlite_openai::edenai_embedding::{
    TransformInput as EmbeddingInput, map_openai_params, plan_embedding,
    transform_response as embedding_transform_response,
};
use betterlite_openai::edenai_responses::{
    PrepareInput, ReplyRequest as ResponsesRequest, SHOULD_FAKE_STREAM, SUPPORTS_NATIVE_WEBSOCKET,
    prepare, transform_response as responses_transform_response,
};
use betterlite_openai::openai_base_config::CatalogCapabilities;
use betterlite_openai::response_events::RawResponse;
use betterlite_prompts::arguments::json::Object;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};
use uuid::Uuid;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;

use super::fixture::Fixture;
use super::input::{Call, CatalogCase, ChatParams, ChatRequest, EmbeddingPlan, Input};

const ADDITIONAL_HEADERS: &str = "additional_headers";
const API_BASE_ENV: &str = "EDENAI_API_BASE";
const API_KEY_ENV: &str = "EDENAI_API_KEY";
const MODEL: &str = "m";
const RESPONSE_COST: &str = "response_cost";

struct FixedReasoning(bool);

impl ReasoningLookup for FixedReasoning {
    type Error = Infallible;

    fn supports_reasoning(&self, _model: &str) -> Result<bool, Self::Error> {
        Ok(self.0)
    }
}

pub(super) fn run(input: &Input) -> HelperResult {
    if !input.costs.is_empty() {
        let steps: Vec<Option<f64>> = input
            .costs
            .iter()
            .map(|payload| money(reported_cost(payload)))
            .collect();
        return Ok(json!({"steps": steps}));
    }
    if !input.cost_bodies.is_empty() {
        let steps: Vec<Option<f64>> = input
            .cost_bodies
            .iter()
            .map(|text| money(reported_cost_from_bytes(text.as_bytes())))
            .collect();
        return Ok(json!({"steps": steps}));
    }
    if !input.cost_headers.is_empty() {
        let steps: Vec<Option<f64>> = input
            .cost_headers
            .iter()
            .map(|headers| money(reported_cost_from_headers(headers)))
            .collect();
        return Ok(json!({"steps": steps}));
    }
    if !input.credentials.is_empty() {
        let steps = report(&input.credentials, credentials_value)?;
        return Ok(json!({"steps": steps}));
    }
    if !input.chat_params.is_empty() {
        let steps = report(&input.chat_params, chat_params_value)?;
        return Ok(json!({"steps": steps}));
    }
    if !input.chat_requests.is_empty() {
        let steps = report(&input.chat_requests, chat_request_value)?;
        return Ok(json!({"steps": steps}));
    }
    if !input.chat_replies.is_empty() {
        let steps = report_text(&input.chat_replies, chat_reply_value)?;
        return Ok(json!({"steps": steps}));
    }
    if !input.chunks.is_empty() {
        let steps = report(&input.chunks, chunk_value)?;
        return Ok(json!({"steps": steps}));
    }
    if !input.models.is_empty() {
        let steps = report(&input.models, catalog_value)?;
        return Ok(json!({"steps": steps}));
    }
    if !input.embedding_plans.is_empty() {
        let steps = report(&input.embedding_plans, embedding_plan_value)?;
        return Ok(json!({"steps": steps}));
    }
    if !input.embedding_replies.is_empty() {
        let steps = report_text(&input.embedding_replies, embedding_reply_value)?;
        return Ok(json!({"steps": steps}));
    }
    if !input.responses_prepare.is_empty() {
        let steps = report(&input.responses_prepare, prepare_value)?;
        return Ok(json!({"steps": steps}));
    }
    if !input.responses_replies.is_empty() {
        let steps = report_text(&input.responses_replies, responses_reply_value)?;
        return Ok(json!({"steps": steps}));
    }
    if !input.messages_environments.is_empty() {
        let steps = report(&input.messages_environments, messages_environment_value)?;
        return Ok(json!({"steps": steps}));
    }
    if !input.messages_replies.is_empty() {
        let steps = report_text(&input.messages_replies, messages_reply_value)?;
        return Ok(json!({"steps": steps}));
    }
    Err(Box::new(HelperError {
        class: String::from("ValueError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: String::from("expected one b6100 variant"),
    }))
}

fn report<T>(
    cases: &[T],
    convert: fn(&T) -> Result<Value, Box<HelperError>>,
) -> Result<Vec<Value>, Box<HelperError>> {
    let mut values = Vec::with_capacity(cases.len());
    for case in cases {
        values.push(convert(case)?);
    }
    Ok(values)
}

fn report_text(
    cases: &[String],
    convert: fn(&str) -> Result<Value, Box<HelperError>>,
) -> Result<Vec<Value>, Box<HelperError>> {
    let mut values = Vec::with_capacity(cases.len());
    for case in cases {
        values.push(convert(case.as_str())?);
    }
    Ok(values)
}

fn key(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn money(cost: Option<Decimal>) -> Option<f64> {
    cost.and_then(|value| value.to_f64())
}

// the caller fills api_key and api_base from the environment and litellm.api_key from the call
fn env_with(environment: &BTreeMap<String, String>, global: Option<&str>) -> EdenAiEnv {
    EdenAiEnv {
        api_key: environment.get(API_KEY_ENV).map(String::as_str).map(key),
        api_base: environment.get(API_BASE_ENV).cloned(),
        global_api_key: global.map(key),
    }
}

fn hidden_cost(hidden: &HiddenParams) -> Option<f64> {
    let Some(HiddenJson::Object(headers)) = hidden.get(ADDITIONAL_HEADERS) else {
        return None;
    };
    let Some(HiddenJson::Number(number)) = headers.get(RESPONSE_COST_HEADER) else {
        return None;
    };
    number.as_f64()
}

fn credentials_value(case: &Call) -> Result<Value, Box<HelperError>> {
    let env = env_with(&case.environment, case.global_api_key.as_deref());
    let explicit = case.api_key.as_deref().map(key);
    let required = match require_api_key(explicit.as_ref(), MODEL, &env) {
        Ok(found) => json!({"key": found.expose_secret(), "status": null}),
        Err(error) => json!({"key": null, "status": error.status_code()}),
    };
    Ok(json!({
        "api_base": resolve_api_base(case.api_base.as_deref(), &env),
        "api_key": resolve_api_key(explicit.as_ref(), &env)
            .map(|found| found.expose_secret().to_owned()),
        "required": required,
    }))
}

fn chat_params_value(case: &ChatParams) -> Result<Value, Box<HelperError>> {
    let catalog = CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    };
    let lookup = FixedReasoning(case.reasoning);
    Ok(json!(supported_params(&lookup, &case.model, &catalog)))
}

fn chat_request_value(case: &ChatRequest) -> Result<Value, Box<HelperError>> {
    let messages: [ChatMessage; 0] = [];
    let environment = BTreeMap::new();
    let downloads = RefuseDownloads;
    let body = transform_request(
        &TransformInput {
            model: &case.model,
            messages: &messages,
            optional_params: &case.optional_params,
            environment: &environment,
            downloads: &downloads,
        },
        Mode::Sync,
    )
    .or_value_error()?;
    serde_json::to_value(body).or_value_error()
}

fn chat_reply_value(body: &str) -> Result<Value, Box<HelperError>> {
    let fixture = Fixture::new()?;
    let mut state = fixture.state()?;
    let env = fixture.env();
    let headers = IndexMap::new();
    let reply = chat_transform_response(ReplyRequest {
        body: body.as_bytes(),
        headers: &headers,
        state: &mut state,
        env: &env,
        input: LogInput {
            messages: HiddenJson::Null,
            data: RawBody::Object(IndexMap::new()),
        },
        options: ReplyOptions {
            json_mode: false,
            provider: Some("edenai"),
            base_model: None,
            identifier: || Uuid::from_u128(1_u128),
            clock: || 1_700_000_000_i64,
        },
    })
    .or_value_error()?;
    Ok(json!({"cost": hidden_cost(&reply.hidden_params)}))
}

fn chunk_value(chunk: &Value) -> Result<Value, Box<HelperError>> {
    let parsed =
        chunk_parser(chunk, || Uuid::from_u128(1_u128), || 1_700_000_000_i64).or_value_error()?;
    let Some(Value::Object(usage)) = parsed.usage else {
        return Ok(json!({"cost": null}));
    };
    Ok(json!({"cost": usage.get("cost").and_then(Value::as_f64)}))
}

fn catalog_value(case: &CatalogCase) -> Result<Value, Box<HelperError>> {
    let env = env_with(&case.environment, None);
    let url = models_url(case.api_base.as_deref(), &env);
    let reply = CatalogReply {
        status: case.status,
        headers: &case.headers,
        text: &case.text,
    };
    match get_models(&reply) {
        Ok(models) => Ok(json!({"url": url, "models": models})),
        Err(error) => Ok(json!({"url": url, "status": error.status_code})),
    }
}

fn embedding_plan_value(case: &EmbeddingPlan) -> Result<Value, Box<HelperError>> {
    let env = env_with(&case.environment, case.global_api_key.as_deref());
    let explicit = case.api_key.as_deref().map(key);
    let plan = plan_embedding(&EmbeddingInput {
        model: &case.model,
        input: &case.input,
        optional_params: &case.optional_params,
        headers: &case.headers,
        api_key: explicit.as_ref(),
        api_base: case.api_base.as_deref(),
        env: &env,
    })
    .or_value_error()?;
    let params = map_openai_params(&case.non_default_params, &case.optional_params);
    Ok(json!({
        "params": params,
        "url": plan.url,
        "headers": plan.headers,
        "body": plan.body,
    }))
}

fn embedding_reply_value(body: &str) -> Result<Value, Box<HelperError>> {
    let reply = embedding_transform_response(body.as_bytes()).or_value_error()?;
    Ok(json!({"cost": hidden_cost(&reply.hidden_params)}))
}

fn prepare_value(case: &Call) -> Result<Value, Box<HelperError>> {
    let env = env_with(&case.environment, case.global_api_key.as_deref());
    let explicit = case.api_key.as_deref().map(key);
    let prepared = prepare(PrepareInput {
        headers: case.headers.clone(),
        api_key: explicit.as_ref(),
        model: MODEL,
        api_base: case.api_base.as_deref(),
        env: &env,
    });
    match prepared {
        Ok(found) => Ok(json!({
            "url": found.url,
            "headers": found.headers,
            "should_fake_stream": SHOULD_FAKE_STREAM,
            "supports_native_websocket": SUPPORTS_NATIVE_WEBSOCKET,
        })),
        Err(error) => Ok(json!({"status": error.status_code()})),
    }
}

fn responses_reply_value(body: &str) -> Result<Value, Box<HelperError>> {
    let fixture = Fixture::new()?;
    let mut state = fixture.state()?;
    let env = fixture.env();
    let headers = IndexMap::new();
    let reply = RawResponse {
        status: 200,
        headers: &headers,
        text: body,
    };
    let transformed = responses_transform_response(ResponsesRequest {
        reply: &reply,
        now: 1_700_000_000_i64,
        state: &mut state,
        env: &env,
    })
    .or_value_error()?;
    let cost = match transformed.additional_headers.get(RESPONSE_COST_HEADER) {
        Some(HiddenJson::Number(number)) => number.as_f64(),
        Some(HiddenJson::Bool(_) | HiddenJson::Null | HiddenJson::Text(_))
        | Some(HiddenJson::List(_) | HiddenJson::Object(_))
        | None => None,
    };
    Ok(json!({"cost": cost}))
}

fn messages_environment_value(case: &Call) -> Result<Value, Box<HelperError>> {
    let env = env_with(&case.environment, case.global_api_key.as_deref());
    let explicit = case.api_key.as_deref().map(key);
    let lookup = case.environment.clone();
    let optional = Object::new();
    let policy = JsonMessagesPolicy {
        descriptor: descriptor(),
    };
    let validated = messages_validate_environment(
        &policy,
        EnvironmentInput {
            headers: case.headers.clone(),
            api_key: explicit.as_ref(),
            model: MODEL,
            env: &env,
            lookup: &lookup,
            optional_params: &optional,
            messages: &[],
        },
    );
    let headers = match validated {
        Ok(found) => found,
        Err(error) => return Ok(json!({"status": error.status_code()})),
    };
    let url = policy
        .complete_url(&MessagesCredentials {
            api_base: case.api_base.as_deref(),
            api_key: None,
            global_api_key: None,
            lookup: &lookup,
        })
        .or_value_error()?;
    Ok(json!({"headers": headers, "url": url}))
}

fn messages_reply_value(body: &str) -> Result<Value, Box<HelperError>> {
    let fixture = Fixture::new()?;
    let mut state = fixture.state()?;
    let limits = ApiLimits::default();
    messages_transform_response(ReplyInput {
        body: body.as_bytes(),
        status: 200,
        limits: &limits,
        state: &mut state,
    })
    .or_value_error()?;
    let cost = match state.model_call_details.get(RESPONSE_COST) {
        Some(HiddenJson::Number(number)) => number.as_f64(),
        Some(HiddenJson::Bool(_) | HiddenJson::Null | HiddenJson::Text(_))
        | Some(HiddenJson::List(_) | HiddenJson::Object(_))
        | None => None,
    };
    Ok(json!({"cost": cost}))
}
