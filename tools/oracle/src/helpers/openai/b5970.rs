mod script;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_openai::watsonx_auth::WatsonxAuth;
use betterlite_openai::watsonx_auth::credentials::{get_api_params, get_watsonx_credentials};
use betterlite_openai::watsonx_auth::environment::WatsonxEnvironment;
use betterlite_openai::watsonx_auth::iam::get_watsonx_iam_url;
use betterlite_openai::watsonx_auth::payload::prepare_payload;
use betterlite_openai::watsonx_auth::types::{WatsonxApiParams, WatsonxParams};
use betterlite_openai::watsonx_auth::urls::{add_api_version_to_url, get_base_url};
use http::header::{HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};
use script::{Requests, Script, environment, error_value, header_json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    env: BTreeMap<String, String>,
    routes: Vec<script::Route>,
    steps: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Step {
    ApiParams {
        params: WatsonxParams,
        #[serde(default)]
        model: Option<String>,
    },
    Credentials {
        params: WatsonxParams,
        #[serde(default)]
        api_key: Option<String>,
        #[serde(default)]
        api_base: Option<String>,
    },
    BaseUrl {
        #[serde(default)]
        api_base: Option<String>,
    },
    ApiVersion {
        url: String,
        #[serde(default)]
        api_version: Option<String>,
    },
    Payload {
        model: String,
        api_params: WatsonxApiParams,
    },
    IamUrl,
    Authorization {
        headers: BTreeMap<String, String>,
        params: WatsonxParams,
        #[serde(default)]
        api_key: Option<String>,
    },
    Generate {
        #[serde(default)]
        api_key: Option<String>,
        #[serde(default)]
        token: Option<String>,
    },
    Advance {
        seconds: f64,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .map_err(|source| {
                Box::new(HelperError {
                    class: "RuntimeError".to_owned(),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: source.to_string(),
                })
            })?;
        Ok(runtime.block_on(execute(input)))
    }
}

async fn execute(input: &Input) -> Value {
    let requests = Requests::default();
    let script = Script::new(&input.routes, Arc::clone(&requests));
    let clock = Arc::new(Mutex::new(0.0_f64));
    let read_clock = Arc::clone(&clock);
    let created = WatsonxAuth::new(
        environment(&input.env),
        script,
        Arc::new(move || *read_clock.lock().unwrap_or_else(PoisonError::into_inner)),
    );
    let Ok(auth) = created else {
        return json!({"init": {"ok": false}, "steps": []});
    };
    let mut steps = Vec::new();
    for step in &input.steps {
        let result = run_step(&auth, &clock, step).await;
        let sent = std::mem::take(&mut *requests.lock().unwrap_or_else(PoisonError::into_inner));
        steps.push(json!({"result": result, "requests": sent}));
    }
    json!({"steps": steps})
}

async fn run_step(auth: &WatsonxAuth<Script>, clock: &Arc<Mutex<f64>>, step: &Step) -> Value {
    match step {
        Step::ApiParams { params, model } => {
            api_params(params.clone(), model.as_deref(), &auth.environment)
        }
        Step::Credentials {
            params,
            api_key,
            api_base,
        } => credentials(
            params.clone(),
            api_key.as_deref(),
            api_base.as_deref(),
            &auth.environment,
        ),
        Step::BaseUrl { api_base } => base_url(api_base.as_deref(), &auth.environment),
        Step::ApiVersion { url, api_version } => {
            json!({"url": add_api_version_to_url(url, api_version.as_deref())})
        }
        Step::Payload { model, api_params } => json!(prepare_payload(model, api_params)),
        Step::IamUrl => json!({"url": get_watsonx_iam_url(&auth.environment)}),
        Step::Authorization {
            headers,
            params,
            api_key,
        } => authorization(auth, headers, params.clone(), api_key.as_deref()).await,
        Step::Generate { api_key, token } => {
            generate(auth, api_key.as_deref(), token.as_deref()).await
        }
        Step::Advance { seconds } => {
            let mut now = clock.lock().unwrap_or_else(PoisonError::into_inner);
            *now = now.mul_add(1.0_f64, *seconds);
            Value::Null
        }
    }
}

fn api_params(mut params: WatsonxParams, model: Option<&str>, env: &WatsonxEnvironment) -> Value {
    match get_api_params(&mut params, model, env) {
        Ok(found) => json!({"api_params": found, "params": params}),
        Err(error) => error_value(&error),
    }
}

fn credentials(
    mut params: WatsonxParams,
    api_key: Option<&str>,
    api_base: Option<&str>,
    env: &WatsonxEnvironment,
) -> Value {
    let key = api_key.map(SecretString::from);
    match get_watsonx_credentials(&mut params, key.as_ref(), api_base, env) {
        Ok(found) => json!({
            "api_key": found.api_key.expose_secret(),
            "api_base": found.api_base,
            "token": found.token.as_ref().map(ExposeSecret::expose_secret),
            "params": params,
        }),
        Err(error) => error_value(&error),
    }
}

fn base_url(api_base: Option<&str>, env: &WatsonxEnvironment) -> Value {
    match get_base_url(api_base, env) {
        Ok(url) => json!({"url": url}),
        Err(error) => error_value(&error),
    }
}

async fn authorization(
    auth: &WatsonxAuth<Script>,
    headers: &BTreeMap<String, String>,
    mut params: WatsonxParams,
    api_key: Option<&str>,
) -> Value {
    let mut header_map = HeaderMap::new();
    for (header_name, header_value) in headers {
        let Ok(name) = HeaderName::try_from(header_name.as_str()) else {
            return json!({"error": "invalid_header_name"});
        };
        let Ok(value) = HeaderValue::try_from(header_value.as_str()) else {
            return json!({"error": "invalid_header_value"});
        };
        header_map.insert(name, value);
    }
    let key = api_key.map(SecretString::from);
    match auth
        .validate_environment(header_map, &mut params, key.as_ref())
        .await
    {
        Ok(resolved) => json!({"headers": header_json(&resolved), "params": params}),
        Err(error) => error_value(&error),
    }
}

async fn generate(auth: &WatsonxAuth<Script>, api_key: Option<&str>, token: Option<&str>) -> Value {
    let key = api_key.map(SecretString::from);
    let given = token.map(SecretString::from);
    match auth
        .generate_watsonx_token(key.as_ref(), given.as_ref())
        .await
    {
        Ok(found) => json!({"token": found.expose_secret()}),
        Err(error) => error_value(&error),
    }
}
