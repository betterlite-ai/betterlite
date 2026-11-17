use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_models::fetch_policy::response::FetchResponse;
use betterlite_models::lemonade_model_metadata::{
    DeploymentCredentials, LemonadeCredentials, ModelInfoClient, ModelInfoRequest,
};
use betterlite_models::ollama_model_metadata::{LiteLlmGlobals, OllamaSettings};
use betterlite_openai::compatible_wire::{NEW_ID, ReplyOptions, system_clock};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_provider::self_hosted_dispatch::lemonade::{get_models, transform_response};
use betterlite_provider::self_hosted_dispatch::ollama_models::{
    OllamaModels, TagsRequest, TagsResponse, TagsTransport,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Map, Value, json};
use tokio::runtime::Runtime;

use super::fixtures::{answer, environment, substituted, upstream_mapped};
use super::input::Case;
use super::input_failure;
use super::wire::request_view;

const AUTHORIZATION: &str = "Authorization";
const LEMONADE_API_BASE: &str = "LEMONADE_API_BASE";
const LEMONADE_API_KEY: &str = "LEMONADE_API_KEY";
const OLLAMA_API_BASE: &str = "OLLAMA_API_BASE";
const OLLAMA_API_KEY: &str = "OLLAMA_API_KEY";
const MODEL_TIMEOUT: Duration = Duration::from_secs(600);

struct ModelClient {
    requests: Mutex<Vec<ModelInfoRequest>>,
    status: u16,
    body: Vec<u8>,
}

impl ModelClient {
    fn first(&self) -> Result<ModelInfoRequest, Box<HelperError>> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .first()
            .cloned()
            .ok_or_else(|| {
                input_failure(String::from("the lemonade models lookup sent no request"))
            })
    }
}

impl ModelInfoClient for ModelClient {
    type Error = std::io::Error;

    fn get(
        &self,
        request: &ModelInfoRequest,
    ) -> impl Future<Output = Result<FetchResponse, Self::Error>> + Send {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        ready(Ok(FetchResponse::new(
            self.status,
            Vec::new(),
            self.body.clone(),
        )))
    }
}

pub(super) fn lemonade_models(runtime: &Runtime, case: &Case) -> HelperResult {
    let env = environment(case);
    let reply = answer(case)?;
    let client = ModelClient {
        requests: Mutex::new(Vec::new()),
        status: reply.status,
        body: reply.body.clone().into_bytes(),
    };
    let credentials = LemonadeCredentials {
        api_base: env.get(LEMONADE_API_BASE).cloned(),
        api_key: env.get(LEMONADE_API_KEY).map(SecretString::from),
        lemonade_key: case.globals.lemonade_key.as_deref().map(SecretString::from),
    };
    let passed_base = case.api_base.as_deref().map(substituted);
    let passed_key = case.api_key.as_deref().map(SecretString::from);
    let deployment = DeploymentCredentials {
        api_base: passed_base.as_deref(),
        api_key: passed_key.as_ref(),
    };
    let models = runtime
        .block_on(get_models(&client, &credentials, deployment, MODEL_TIMEOUT))
        .map_err(|error| input_failure(format!("the lemonade models lookup failed: {error}")))?;
    let recorded = client.first()?;
    Ok(json!({
        "models": models,
        "request": request_view(
            &upstream_mapped(&recorded.url),
            authorization(&recorded.headers),
        ),
    }))
}

pub(super) fn lemonade_reply(case: &Case) -> HelperResult {
    let reply = answer(case)?;
    let options = ReplyOptions {
        json_mode: false,
        provider: None,
        base_model: None,
        identifier: NEW_ID,
        clock: system_clock,
    };
    let decoded = transform_response(reply.body.as_bytes(), &case.model, &options)
        .map_err(|error| input_failure(format!("the lemonade reply could not be read: {error}")))?;
    Ok(json!({"model": decoded.response.model}))
}

struct TagsRecorder {
    requests: Mutex<Vec<TagsRequest>>,
    status: u16,
    body: Vec<u8>,
}

impl TagsRecorder {
    fn first(&self) -> Result<TagsRequest, Box<HelperError>> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .first()
            .cloned()
            .ok_or_else(|| input_failure(String::from("the ollama models lookup sent no request")))
    }
}

impl TagsTransport for TagsRecorder {
    type Error = std::io::Error;

    fn get_tags(&self, request: &TagsRequest) -> Result<TagsResponse, Self::Error> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        Ok(TagsResponse {
            status: self.status,
            body: self.body.clone(),
        })
    }
}

pub(super) fn ollama_models(case: &Case) -> HelperResult {
    let env = environment(case);
    let reply = answer(case)?;
    let transport = TagsRecorder {
        requests: Mutex::new(Vec::new()),
        status: reply.status,
        body: reply.body.clone().into_bytes(),
    };
    let settings = OllamaSettings {
        api_key: env.get(OLLAMA_API_KEY).map(SecretString::from),
        api_base: env.get(OLLAMA_API_BASE).cloned(),
    };
    let globals = LiteLlmGlobals {
        api_key: case.globals.api_key.as_deref().map(SecretString::from),
        openai_key: case.globals.openai_key.as_deref().map(SecretString::from),
        secret_manager_api_key: case
            .globals
            .secret_manager_api_key
            .as_deref()
            .map(SecretString::from),
    };
    let passed_base = case.api_base.as_deref().map(substituted);
    let passed_key = case.api_key.as_deref().map(SecretString::from);
    let models = OllamaModels {
        settings: &settings,
        globals: &globals,
        catalog: &case.catalog,
        transport: &transport,
    }
    .get_models(passed_base.as_deref(), passed_key.as_ref());
    let recorded = transport.first()?;
    Ok(json!({
        "models": models,
        "request": request_view(
            &upstream_mapped(&recorded.url),
            authorization(&recorded.headers),
        ),
    }))
}

// the only header either listing prepares itself; its value already carries the bearer prefix
fn authorization(headers: &[(&'static str, SecretString)]) -> Value {
    let mut kept = Map::new();
    for (name, value) in headers {
        if *name == AUTHORIZATION {
            kept.insert(
                String::from("authorization"),
                Value::String(value.expose_secret().to_owned()),
            );
        }
    }
    Value::Object(kept)
}
