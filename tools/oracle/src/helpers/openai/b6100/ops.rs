use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;

use betterlite_core::edenai_messages::{
    EnvironmentInput, descriptor, validate_environment as messages_validate_environment,
};
use betterlite_core::json_messages_policy::{JsonMessagesPolicy, MessagesCredentials};
use betterlite_openai::chat_content::{Mode, RefuseDownloads};
use betterlite_openai::edenai_chat::{
    CatalogReply, ReasoningLookup, TransformInput, get_models, models_url, supported_params,
    transform_request,
};
use betterlite_openai::edenai_common::{
    EdenAiEnv, reported_cost, reported_cost_from_bytes, reported_cost_from_headers,
    require_api_key, resolve_api_base, resolve_api_key,
};
use betterlite_openai::edenai_embedding::{
    EdenAiEmbeddingError, EmbeddingPlan as Plan, TransformInput as EmbeddingInput,
    map_openai_params, plan_embedding,
};
use betterlite_openai::edenai_responses::{
    PrepareInput, SHOULD_FAKE_STREAM, SUPPORTS_NATIVE_WEBSOCKET, prepare,
};
use betterlite_openai::openai_base_config::CatalogCapabilities;
use betterlite_prompts::arguments::json::Object;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;

use super::input::{Call, CatalogCase, ChatRequest, EmbeddingPlan, Input};
use super::replies::reply_value;
use super::steps;

const API_BASE_ENV: &str = "EDENAI_API_BASE";
const API_KEY_ENV: &str = "EDENAI_API_KEY";
const MODEL: &str = "m";
const NO_DOWNLOADS: RefuseDownloads = RefuseDownloads;

struct FixedReasoning(bool);

impl ReasoningLookup for FixedReasoning {
    type Error = Infallible;

    fn supports_reasoning(&self, _model: &str) -> Result<bool, Self::Error> {
        Ok(self.0)
    }
}

// one case carries one variant, so each branch answers the whole call the case makes
pub(super) fn run(input: &Input) -> HelperResult {
    if !input.costs.is_empty() || !input.cost_bodies.is_empty() || !input.cost_headers.is_empty() {
        return Ok(cost_value(input));
    }
    if !input.credentials.is_empty() || !input.chat_params.is_empty() {
        return Ok(call_value(input));
    }
    if !input.chat_requests.is_empty() {
        return steps(&input.chat_requests, chat_request_step);
    }
    if !input.models.is_empty() {
        return steps(&input.models, model_step);
    }
    if !input.embedding_plans.is_empty() {
        return steps(&input.embedding_plans, embedding_step);
    }
    if !input.responses_prepare.is_empty() {
        return call_steps(&input.responses_prepare, CallFamily::Responses);
    }
    if !input.messages_environments.is_empty() {
        return call_steps(&input.messages_environments, CallFamily::Messages);
    }
    if let Some(answer) = reply_value(input)? {
        return Ok(answer);
    }
    Err(missing_variant())
}

// the three reported-cost shapes in one family, cheapest first as LiteLLM reads them
fn cost_value(input: &Input) -> Value {
    if !input.costs.is_empty() {
        return cost_steps(input.costs.iter().map(reported_cost));
    }
    if !input.cost_bodies.is_empty() {
        return cost_steps(
            input
                .cost_bodies
                .iter()
                .map(|text| reported_cost_from_bytes(text.as_bytes())),
        );
    }
    cost_steps(input.cost_headers.iter().map(reported_cost_from_headers))
}

fn cost_steps<Costs: Iterator<Item = Option<Decimal>>>(costs: Costs) -> Value {
    let steps: Vec<Option<f64>> = costs
        .map(|cost| cost.and_then(|value| value.to_f64()))
        .collect();
    json!({"steps": steps})
}

// the credential resolution and the supported-parameter list, both read from the caller's values
fn call_value(input: &Input) -> Value {
    if !input.credentials.is_empty() {
        let mut steps = Vec::with_capacity(input.credentials.len());
        for case in &input.credentials {
            let env = env_with(&case.environment, case.global_api_key.as_deref());
            let explicit = case.api_key.clone().map(SecretString::from);
            let required = match require_api_key(explicit.as_ref(), MODEL, &env) {
                Ok(found) => json!({"key": found.expose_secret(), "status": null}),
                Err(error) => json!({"key": null, "status": error.status_code}),
            };
            steps.push(json!({
                "api_base": resolve_api_base(case.api_base.as_deref(), &env),
                "api_key": resolve_api_key(explicit.as_ref(), &env)
                    .map(|found| found.expose_secret().to_owned()),
                "required": required,
            }));
        }
        return json!({"steps": steps});
    }
    let catalog = CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    };
    let mut steps = Vec::with_capacity(input.chat_params.len());
    for case in &input.chat_params {
        let lookup = FixedReasoning(case.reasoning);
        steps.push(json!(supported_params(&lookup, &case.model, &catalog)));
    }
    json!({"steps": steps})
}

// the request bodies are the one family whose transform can fail
fn chat_request_step(case: &ChatRequest) -> Result<Value, Box<HelperError>> {
    let body = transform_request(&chat_input(case), Mode::Sync).or_value_error()?;
    serde_json::to_value(body).or_value_error()
}

fn chat_input(case: &ChatRequest) -> TransformInput<'_> {
    TransformInput {
        model: &case.model,
        messages: &[],
        optional_params: &case.optional_params,
        environment: &case.environment,
        downloads: &NO_DOWNLOADS,
    }
}

// the catalog reply: a failed status keeps the status, and a good one carries the prefixed ids
fn model_step(case: &CatalogCase) -> Result<Value, Box<HelperError>> {
    let env = env_with(&case.environment, None);
    let url = models_url(case.api_base.as_deref(), &env);
    let reply = CatalogReply {
        status: case.status,
        headers: &case.headers,
        text: &case.text,
    };
    Ok(match get_models(&reply) {
        Ok(models) => json!({"url": url, "models": models}),
        Err(error) => json!({"url": url, "status": error.status_code}),
    })
}

fn embedding_step(case: &EmbeddingPlan) -> Result<Value, Box<HelperError>> {
    let plan = embedding_plan(case).or_value_error()?;
    let params = map_openai_params(&case.non_default_params, &case.optional_params);
    Ok(json!({"params": params, "url": plan.url, "headers": plan.headers, "body": plan.body}))
}

fn embedding_plan(case: &EmbeddingPlan) -> Result<Plan, EdenAiEmbeddingError> {
    let env = env_with(&case.environment, case.global_api_key.as_deref());
    let explicit = case.api_key.clone().map(SecretString::from);
    plan_embedding(&EmbeddingInput {
        model: &case.model,
        input: &case.input,
        optional_params: &case.optional_params,
        headers: &case.headers,
        api_key: explicit.as_ref(),
        api_base: case.api_base.as_deref(),
        env: &env,
    })
}

#[derive(Clone, Copy)]
enum CallFamily {
    Responses,
    Messages,
}

// the two call families take the same case shape: Responses prepares its plan, Messages validates its environment
fn call_step(case: &Call, family: CallFamily) -> Result<Value, Box<HelperError>> {
    let env = env_with(&case.environment, case.global_api_key.as_deref());
    let explicit = case.api_key.clone().map(SecretString::from);
    if let CallFamily::Messages = family {
        return messages_answer(case, &env, explicit.as_ref());
    }
    let prepared = prepare(PrepareInput {
        headers: case.headers.clone(),
        api_key: explicit.as_ref(),
        model: MODEL,
        api_base: case.api_base.as_deref(),
        env: &env,
    });
    Ok(match prepared {
        Ok(found) => json!({
            "url": found.url,
            "headers": found.headers,
            "should_fake_stream": SHOULD_FAKE_STREAM,
            "supports_native_websocket": SUPPORTS_NATIVE_WEBSOCKET,
        }),
        Err(error) => json!({"status": error.status_code}),
    })
}

// a rejected environment keeps its status in the case's step, and the other cases still run
fn messages_answer(
    case: &Call,
    env: &EdenAiEnv,
    explicit: Option<&SecretString>,
) -> Result<Value, Box<HelperError>> {
    let policy = JsonMessagesPolicy {
        descriptor: descriptor(),
    };
    let validated = messages_validate_environment(
        &policy,
        EnvironmentInput {
            headers: case.headers.clone(),
            api_key: explicit,
            model: MODEL,
            env,
            lookup: &case.environment,
            optional_params: &Object::new(),
            messages: &[],
        },
    );
    let headers = match validated {
        Ok(found) => found,
        Err(error) => return Ok(json!({"status": error.status_code})),
    };
    let url = policy
        .complete_url(&MessagesCredentials {
            api_base: case.api_base.as_deref(),
            api_key: None,
            global_api_key: None,
            lookup: &case.environment,
        })
        .or_value_error()?;
    Ok(json!({"headers": headers, "url": url}))
}

fn call_steps(cases: &[Call], family: CallFamily) -> Result<Value, Box<HelperError>> {
    let mut values = Vec::with_capacity(cases.len());
    for case in cases {
        values.push(call_step(case, family)?);
    }
    Ok(json!({"steps": values}))
}

fn missing_variant() -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("ValueError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: String::from("expected one b6100 variant"),
    })
}

// the caller fills api_key and api_base from the environment and litellm.api_key from the call
fn env_with(environment: &BTreeMap<String, String>, global: Option<&str>) -> EdenAiEnv {
    let api_key = environment.get(API_KEY_ENV).cloned();
    EdenAiEnv {
        api_key: api_key.map(SecretString::from),
        api_base: environment.get(API_BASE_ENV).cloned(),
        global_api_key: global.map(str::to_owned).map(SecretString::from),
    }
}
