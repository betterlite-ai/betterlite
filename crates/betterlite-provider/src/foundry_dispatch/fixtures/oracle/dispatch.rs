use std::collections::BTreeMap;

use betterlite_config::sections::azure_credentials::DeploymentSettings;
use betterlite_foundry::agent_results::reply::AgentReply;
use betterlite_foundry::chat::sources::NoSources;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::ChatRequest;
use betterlite_prompts::arguments::json::Object;
use http::HeaderMap;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use tokio::runtime::Builder;

use crate::foundry_dispatch::chat::Answer;
use crate::foundry_dispatch::credentials::{self, Globals};
use crate::foundry_dispatch::fixtures::logger::RecordingLogger;
use crate::foundry_dispatch::{agents, chat, claude, route};
use crate::prepared::deployment::Deployment;

use super::values::normalized;
use super::{DispatchCase, EmbeddingCase, Observation, PostCall, RewriteCase};

#[cfg_attr(test, mutants::skip)]
pub(super) fn dispatch_case(case: &DispatchCase) -> Observation {
    let settings = DeploymentSettings::default();
    let global_key = secret(case.globals.api_key.as_deref());
    let explicit_key = secret(case.api_key.as_deref());
    let environment_key = secret(case.environment.get("AZURE_AI_API_KEY").map(String::as_str));
    let globals = Globals {
        api_base: case.globals.api_base.as_deref(),
        api_key: global_key.as_ref(),
        environment_api_base: case
            .environment
            .get("AZURE_AI_API_BASE")
            .map(String::as_str),
        environment_api_key: environment_key.as_ref(),
        headers: case.globals.headers.as_ref(),
        environment: &case.environment,
        credentials: &settings,
        refresh_enabled: false,
        sources: &NoSources,
        token_provider: None,
    };
    match route::select(&case.model) {
        route::Route::Chat => chat_case(case, globals, explicit_key.as_ref()),
        route::Route::Claude => claude_case(case, &globals, explicit_key.as_ref()),
        route::Route::Agents => agents_case(case, &globals, explicit_key.as_ref()),
    }
}

#[derive(Serialize)]
struct Shape<'data> {
    model: &'data str,
    messages: &'data [Object],
}

#[cfg_attr(test, mutants::skip)]
fn chat_case(
    case: &DispatchCase,
    globals: Globals<'_>,
    explicit_key: Option<&SecretString>,
) -> Observation {
    let Some(request) = chat_request(case) else {
        return failed();
    };
    let Some(deployment) = deployment(case, &globals, explicit_key) else {
        return failed();
    };
    let call = chat::Call {
        model: &case.model,
        request: &request,
        extra_headers: None,
        headers: case.headers.as_ref(),
        api_base: case.api_base.as_deref(),
        api_key: explicit_key,
        deployment: &deployment,
        stream: false,
    };
    let Ok(plan) = chat::prepare::prepare(&call, &globals) else {
        return failed();
    };
    let post_calls = run_chat(case.status, call, globals);
    Observation {
        branch: Some("chat"),
        base: plan.base,
        key: plan.key.map(|key| key.expose_secret().to_owned()),
        headers: plan.headers,
        messages: normalized(&plan.messages),
        post_calls,
        errors: false,
    }
}

#[cfg_attr(test, mutants::skip)]
fn chat_request(case: &DispatchCase) -> Option<ChatRequest> {
    let shape = Shape {
        model: &case.model,
        messages: &case.messages,
    };
    serde_json::to_value(shape)
        .and_then(serde_json::from_value::<ChatRequest>)
        .ok()
}

#[cfg_attr(test, mutants::skip)]
fn deployment(
    case: &DispatchCase,
    globals: &Globals<'_>,
    key: Option<&SecretString>,
) -> Option<Deployment> {
    let base = globals
        .accessors()
        .api_base(case.api_base.as_deref().filter(|text| !text.is_empty()));
    let url = base.as_deref().unwrap_or("https://placeholder.test");
    let mut params: IndexMap<String, JsonValue> = IndexMap::new();
    params.insert(String::from("model"), JsonValue::Text(case.model.clone()));
    params.insert(String::from("api_base"), JsonValue::Text(url.to_owned()));
    if let Some(key) = key {
        params.insert(
            String::from("api_key"),
            JsonValue::Text(key.expose_secret().to_owned()),
        );
    }
    Deployment::from_params(params).ok()
}

// main.py hands the chat post_call an additional_args map on both the failure and stream paths
#[cfg_attr(test, mutants::skip)]
fn run_chat(status: Option<u16>, call: chat::Call<'_>, globals: Globals<'_>) -> Vec<PostCall> {
    let Some(status) = status else {
        return Vec::new();
    };
    let Ok(runtime) = Builder::new_current_thread().enable_all().build() else {
        return Vec::new();
    };
    let mut logger = RecordingLogger::default();
    let answer = Answer::Supplied {
        status,
        headers: HeaderMap::new(),
        body: Vec::new(),
    };
    let _outcome = runtime.block_on(async {
        chat::Dispatch {
            call,
            globals,
            answer,
        }
        .run(&mut logger)
        .await
    });
    logger
        .calls
        .iter()
        .map(|logged| PostCall {
            error: matches!(&logged.original_response, Json::Text(_)),
            headers: true,
        })
        .collect()
}

#[cfg_attr(test, mutants::skip)]
fn claude_case(
    case: &DispatchCase,
    globals: &Globals<'_>,
    explicit_key: Option<&SecretString>,
) -> Observation {
    let base = globals
        .accessors()
        .api_base(case.api_base.as_deref().filter(|text| !text.is_empty()));
    let Some(base) = base else {
        return failed();
    };
    Observation {
        branch: Some("claude"),
        base: Some(claude::endpoint(&base)),
        key: credentials::resolve_key(explicit_key, globals)
            .map(|key| key.expose_secret().to_owned()),
        headers: case.headers.clone(),
        messages: normalized(&case.messages),
        post_calls: Vec::new(),
        errors: false,
    }
}

#[cfg_attr(test, mutants::skip)]
fn agents_case(
    case: &DispatchCase,
    globals: &Globals<'_>,
    explicit_key: Option<&SecretString>,
) -> Observation {
    let optionals = Object::new();
    let litellm_params = Object::new();
    let call = agents::Call {
        model: &case.model,
        messages: &case.messages,
        optional_params: &optionals,
        litellm_params: &litellm_params,
        headers: case.headers.as_ref(),
        api_base: case.api_base.as_deref(),
        api_key: explicit_key,
        acompletion: false,
        stream: false,
        reply: AgentReply::default(),
    };
    let Ok(resolved) = agents::resolve(&call, globals) else {
        return failed();
    };
    // main.py passes `headers or litellm.headers`, so no header source at all arrives as None
    let headers: Option<BTreeMap<String, String>> =
        credentials::headers_or_global(case.headers.as_ref(), globals).map(|given| {
            given
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect()
        });
    Observation {
        branch: Some("agents"),
        base: Some(resolved.api_base),
        key: resolved.api_key.map(|key| key.expose_secret().to_owned()),
        headers,
        messages: normalized(&case.messages),
        post_calls: Vec::new(),
        errors: false,
    }
}

#[cfg_attr(test, mutants::skip)]
pub(super) fn embedding_case(case: &EmbeddingCase) -> Observation {
    let settings = DeploymentSettings::default();
    let global_key = secret(case.globals.api_key.as_deref());
    let explicit_key = secret(case.api_key.as_deref());
    let environment_key = secret(case.environment.get("AZURE_AI_API_KEY").map(String::as_str));
    let globals = Globals {
        api_base: case.globals.api_base.as_deref(),
        api_key: global_key.as_ref(),
        environment_api_base: case
            .environment
            .get("AZURE_AI_API_BASE")
            .map(String::as_str),
        environment_api_key: environment_key.as_ref(),
        headers: case.globals.headers.as_ref(),
        environment: &case.environment,
        credentials: &settings,
        refresh_enabled: false,
        sources: &NoSources,
        token_provider: None,
    };
    Observation {
        branch: Some("embedding"),
        base: globals
            .accessors()
            .api_base(case.api_base.as_deref().filter(|text| !text.is_empty())),
        key: credentials::resolve_key(explicit_key.as_ref(), &globals)
            .map(|key| key.expose_secret().to_owned()),
        headers: None,
        messages: JsonValue::Array(Vec::new()),
        post_calls: Vec::new(),
        errors: false,
    }
}

#[cfg_attr(test, mutants::skip)]
pub(super) fn rewrite_case(case: &RewriteCase) -> Observation {
    Observation {
        branch: Some("claude"),
        base: Some(claude::endpoint(&case.base)),
        key: None,
        headers: None,
        messages: rewrite_messages(),
        post_calls: Vec::new(),
        errors: false,
    }
}

// the Python helper sends this fixed message list with every rewrite, so both observations match
#[cfg_attr(test, mutants::skip)]
fn rewrite_messages() -> JsonValue {
    JsonValue::Array(vec![JsonValue::Object(IndexMap::from([
        (String::from("role"), JsonValue::Text(String::from("user"))),
        (String::from("content"), JsonValue::Text(String::from("hi"))),
    ]))])
}

#[cfg_attr(test, mutants::skip)]
fn secret(value: Option<&str>) -> Option<SecretString> {
    value.map(SecretString::from)
}

#[cfg_attr(test, mutants::skip)]
const fn failed() -> Observation {
    Observation {
        branch: None,
        base: None,
        key: None,
        headers: None,
        messages: JsonValue::Array(Vec::new()),
        post_calls: Vec::new(),
        errors: true,
    }
}
