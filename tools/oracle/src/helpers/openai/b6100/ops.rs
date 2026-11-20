use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;

use betterlite_core::edenai_messages::{
    EnvironmentInput, descriptor, validate_environment as messages_validate_environment,
};
use betterlite_core::json_messages_policy::{JsonMessagesPolicy, MessagesCredentials};
use betterlite_openai::chat::ChatMessage;
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
    TransformInput as EmbeddingInput, map_openai_params, plan_embedding,
};
use betterlite_openai::edenai_responses::{
    PrepareInput, SHOULD_FAKE_STREAM, SUPPORTS_NATIVE_WEBSOCKET, prepare,
};
use betterlite_openai::openai_base_config::CatalogCapabilities;
use betterlite_prompts::arguments::json::Object;
use rust_decimal::prelude::ToPrimitive as _;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;

use super::input::{Call, CatalogCase, Input};
use super::replies::reply_value;

pub(super) const API_BASE_ENV: &str = "EDENAI_API_BASE";
pub(super) const API_KEY_ENV: &str = "EDENAI_API_KEY";
pub(super) const MODEL: &str = "m";

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
        return chat_request_values(input);
    }
    if !input.models.is_empty() {
        return Ok(model_values(&input.models));
    }
    if !input.embedding_plans.is_empty() {
        return embedding_values(input);
    }
    if !input.responses_prepare.is_empty() {
        return Ok(responses_values(&input.responses_prepare));
    }
    if !input.messages_environments.is_empty() {
        return messages_values(input);
    }
    if let Some(answer) = reply_value(input)? {
        return Ok(answer);
    }
    Err(missing_variant())
}

// the three reported-cost shapes in one family, cheapest first as LiteLLM reads them
fn cost_value(input: &Input) -> Value {
    if !input.costs.is_empty() {
        let steps: Vec<Option<f64>> = input
            .costs
            .iter()
            .map(|payload| reported_cost(payload).and_then(|value| value.to_f64()))
            .collect();
        return json!({"steps": steps});
    }
    if !input.cost_bodies.is_empty() {
        let steps: Vec<Option<f64>> = input
            .cost_bodies
            .iter()
            .map(|text| reported_cost_from_bytes(text.as_bytes()).and_then(|value| value.to_f64()))
            .collect();
        return json!({"steps": steps});
    }
    let steps: Vec<Option<f64>> = input
        .cost_headers
        .iter()
        .map(|headers| reported_cost_from_headers(headers).and_then(|value| value.to_f64()))
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
fn chat_request_values(input: &Input) -> HelperResult {
    let mut steps = Vec::with_capacity(input.chat_requests.len());
    for case in &input.chat_requests {
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
        steps.push(serde_json::to_value(body).or_value_error()?);
    }
    Ok(json!({"steps": steps}))
}

// the catalog reply: a failed status keeps the status, and a good one carries the prefixed ids
fn model_values(cases: &[CatalogCase]) -> Value {
    let mut steps = Vec::with_capacity(cases.len());
    for case in cases {
        let env = env_with(&case.environment, None);
        let url = models_url(case.api_base.as_deref(), &env);
        let reply = CatalogReply {
            status: case.status,
            headers: &case.headers,
            text: &case.text,
        };
        steps.push(match get_models(&reply) {
            Ok(models) => json!({"url": url, "models": models}),
            Err(error) => json!({"url": url, "status": error.status_code}),
        });
    }
    json!({"steps": steps})
}

fn embedding_values(input: &Input) -> HelperResult {
    let mut steps = Vec::with_capacity(input.embedding_plans.len());
    for case in &input.embedding_plans {
        let env = env_with(&case.environment, case.global_api_key.as_deref());
        let explicit = case.api_key.clone().map(SecretString::from);
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
        steps.push(json!({
            "params": map_openai_params(&case.non_default_params, &case.optional_params),
            "url": plan.url,
            "headers": plan.headers,
            "body": plan.body,
        }));
    }
    Ok(json!({"steps": steps}))
}

// the Responses plan carries the two API capabilities beside its URL and headers
fn responses_values(cases: &[Call]) -> Value {
    let mut steps = Vec::with_capacity(cases.len());
    for case in cases {
        let env = env_with(&case.environment, case.global_api_key.as_deref());
        let explicit = case.api_key.clone().map(SecretString::from);
        let prepared = prepare(PrepareInput {
            headers: case.headers.clone(),
            api_key: explicit.as_ref(),
            model: MODEL,
            api_base: case.api_base.as_deref(),
            env: &env,
        });
        steps.push(match prepared {
            Ok(found) => json!({
                "url": found.url,
                "headers": found.headers,
                "should_fake_stream": SHOULD_FAKE_STREAM,
                "supports_native_websocket": SUPPORTS_NATIVE_WEBSOCKET,
            }),
            Err(error) => json!({"status": error.status_code}),
        });
    }
    json!({"steps": steps})
}

// a rejected environment keeps its status in the case's step, and the rest of the call still runs
fn messages_values(input: &Input) -> HelperResult {
    let policy = JsonMessagesPolicy {
        descriptor: descriptor(),
    };
    let mut steps = Vec::with_capacity(input.messages_environments.len());
    for case in &input.messages_environments {
        let env = env_with(&case.environment, case.global_api_key.as_deref());
        let explicit = case.api_key.clone().map(SecretString::from);
        let validated = messages_validate_environment(
            &policy,
            EnvironmentInput {
                headers: case.headers.clone(),
                api_key: explicit.as_ref(),
                model: MODEL,
                env: &env,
                lookup: &case.environment,
                optional_params: &Object::new(),
                messages: &[],
            },
        );
        let headers = match validated {
            Ok(found) => found,
            Err(error) => {
                steps.push(json!({"status": error.status_code}));
                continue;
            }
        };
        let url = policy
            .complete_url(&MessagesCredentials {
                api_base: case.api_base.as_deref(),
                api_key: None,
                global_api_key: None,
                lookup: &case.environment,
            })
            .or_value_error()?;
        steps.push(json!({"headers": headers, "url": url}));
    }
    Ok(json!({"steps": steps}))
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
pub(super) fn env_with(environment: &BTreeMap<String, String>, global: Option<&str>) -> EdenAiEnv {
    EdenAiEnv {
        api_key: environment
            .get(API_KEY_ENV)
            .cloned()
            .map(SecretString::from),
        api_base: environment.get(API_BASE_ENV).cloned(),
        global_api_key: global.map(str::to_owned).map(SecretString::from),
    }
}
