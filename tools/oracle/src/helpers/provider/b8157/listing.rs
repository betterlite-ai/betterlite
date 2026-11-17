use std::future::{Future, ready};
use std::io::Error as IoError;
use std::marker::PhantomData;
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

use super::fixtures::{ORACLE_UPSTREAM, answer, environment};
use super::input::Case;
use super::input_failure;

const AUTHORIZATION: &str = "Authorization";
const LEMONADE_API_BASE: &str = "LEMONADE_API_BASE";
const LEMONADE_API_KEY: &str = "LEMONADE_API_KEY";
const OLLAMA_API_BASE: &str = "OLLAMA_API_BASE";
const OLLAMA_API_KEY: &str = "OLLAMA_API_KEY";
const MODEL_TIMEOUT: Duration = Duration::from_secs(600);

// the request a listing sent, as both listings answer with it
#[derive(Clone)]
struct Recorded {
    url: String,
    headers: Vec<(&'static str, SecretString)>,
}

// the upstream both listings call, answering every request with the case's reply and keeping what it was sent
struct Recorder<Request> {
    requests: Mutex<Vec<Recorded>>,
    status: u16,
    body: Vec<u8>,
    request: PhantomData<Request>,
}

impl<Request> Recorder<Request> {
    // the case's own answer, which the recorder sends to every request the lookup makes
    fn for_case(case: &Case) -> Result<Self, Box<HelperError>> {
        let reply = answer(case)?;
        Ok(Self {
            requests: Mutex::new(Vec::new()),
            status: reply.status,
            body: reply.body.clone().into_bytes(),
            request: PhantomData,
        })
    }

    fn push(&self, url: &str, headers: &[(&'static str, SecretString)]) {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded {
                url: url.to_owned(),
                headers: headers.to_vec(),
            });
    }

    fn first(&self, missing: &'static str) -> Result<Recorded, Box<HelperError>> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .first()
            .cloned()
            .ok_or_else(|| input_failure(String::from(missing)))
    }
}

impl ModelInfoClient for Recorder<ModelInfoRequest> {
    type Error = IoError;

    fn get(
        &self,
        request: &ModelInfoRequest,
    ) -> impl Future<Output = Result<FetchResponse, Self::Error>> + Send {
        self.push(&request.url, &request.headers);
        ready(Ok(FetchResponse::new(
            self.status,
            Vec::new(),
            self.body.clone(),
        )))
    }
}

impl TagsTransport for Recorder<TagsRequest> {
    type Error = IoError;

    fn get_tags(&self, request: &TagsRequest) -> Result<TagsResponse, Self::Error> {
        self.push(&request.url, &request.headers);
        Ok(TagsResponse {
            status: self.status,
            body: self.body.clone(),
        })
    }
}

// the base and key the case passed, with the placeholder mapped back to the socket the case names
fn passed(case: &Case) -> (Option<String>, Option<SecretString>) {
    let base = case
        .api_base
        .as_deref()
        .map(|value| value.replace("{{upstream}}", ORACLE_UPSTREAM));
    (base, case.api_key.as_deref().map(SecretString::from))
}

fn lemonade_credentials(case: &Case) -> LemonadeCredentials {
    let env = environment(case);
    LemonadeCredentials {
        api_base: env.get(LEMONADE_API_BASE).cloned(),
        api_key: env.get(LEMONADE_API_KEY).cloned().map(SecretString::from),
        lemonade_key: case.globals.lemonade_key.as_deref().map(SecretString::from),
    }
}

pub(super) fn lemonade_models(runtime: &Runtime, case: &Case) -> HelperResult {
    let client = Recorder::<ModelInfoRequest>::for_case(case)?;
    let (passed_base, passed_key) = passed(case);
    let models = runtime
        .block_on(get_models(
            &client,
            &lemonade_credentials(case),
            DeploymentCredentials {
                api_base: passed_base.as_deref(),
                api_key: passed_key.as_ref(),
            },
            MODEL_TIMEOUT,
        ))
        .map_err(|error| input_failure(format!("the lemonade models lookup failed: {error}")))?;
    Ok(listed(
        &models,
        &client.first("the lemonade models lookup sent no request")?,
    ))
}

fn reply_options() -> ReplyOptions<'static> {
    ReplyOptions {
        json_mode: false,
        provider: None,
        base_model: None,
        identifier: NEW_ID,
        clock: system_clock,
    }
}

pub(super) fn lemonade_reply(case: &Case) -> HelperResult {
    let reply = answer(case)?;
    let decoded = transform_response(reply.body.as_bytes(), &case.model, &reply_options())
        .map_err(|error| input_failure(format!("the lemonade reply could not be read: {error}")))?;
    Ok(json!({"model": decoded.response.model}))
}

pub(super) fn ollama_models(case: &Case) -> HelperResult {
    let transport = Recorder::<TagsRequest>::for_case(case)?;
    let models = ollama_names(case, &transport);
    Ok(listed(
        &models,
        &transport.first("the ollama models lookup sent no request")?,
    ))
}

fn ollama_names(case: &Case, transport: &Recorder<TagsRequest>) -> Vec<String> {
    let env = environment(case);
    let settings = OllamaSettings {
        api_key: env.get(OLLAMA_API_KEY).cloned().map(SecretString::from),
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
    let (passed_base, passed_key) = passed(case);
    OllamaModels {
        settings: &settings,
        globals: &globals,
        catalog: &case.catalog,
        transport,
    }
    .get_models(passed_base.as_deref(), passed_key.as_ref())
}

// a listing answer names the socket the case wrote, since the python side only knows its own
fn listed(models: &[String], recorded: &Recorded) -> Value {
    json!({
        "models": models,
        "request": {
            "url": recorded.url.replace(ORACLE_UPSTREAM, "{{upstream}}"),
            "headers": authorization(&recorded.headers),
        },
    })
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
