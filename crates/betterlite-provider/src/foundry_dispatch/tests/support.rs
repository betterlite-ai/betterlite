use std::collections::BTreeMap;
use std::future::Future;

use betterlite_config::sections::azure_credentials::DeploymentSettings;
use betterlite_foundry::chat::sources::NoSources;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::openai_base_config::Headers;
use http::HeaderMap;
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio::runtime::Builder;

use crate::foundry_dispatch::chat::Answer;
use crate::foundry_dispatch::credentials::Globals;
use crate::prepared::deployment::Deployment;

pub const KEY: &str = "sk-foundry-fixture";

pub fn field<'value>(value: &'value JsonValue, name: &str) -> &'value JsonValue {
    let JsonValue::Object(fields) = value else {
        panic!("the value is a JSON object");
    };
    fields.get(name).expect("the field is present")
}

pub fn block_on<F>(future: F) -> F::Output
where
    F: Future,
{
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the runtime builds")
        .block_on(future)
}

pub fn supplied(status: u16, body: &str) -> Answer<'static> {
    Answer::Supplied {
        status,
        headers: HeaderMap::new(),
        body: body.as_bytes().to_vec(),
    }
}

pub fn upstream(status: u16, body: &str) -> UpstreamResponse {
    UpstreamResponse {
        status,
        headers: HeaderMap::new(),
        body: body.as_bytes().to_vec(),
    }
}

pub fn request(body: &str) -> ChatRequest {
    serde_json::from_str(body).expect("a chat request")
}

pub fn simple_request(model: &str) -> ChatRequest {
    request(&format!(
        r#"{{"model":"{model}","messages":[{{"role":"user","content":"hi"}}]}}"#
    ))
}

pub fn deployment(api_base: &str, api_key: Option<&str>) -> Deployment {
    let mut params: IndexMap<String, JsonValue> = IndexMap::new();
    params.insert(
        String::from("model"),
        JsonValue::Text(String::from("deployment")),
    );
    params.insert(
        String::from("api_base"),
        JsonValue::Text(api_base.to_owned()),
    );
    if let Some(key) = api_key {
        params.insert(String::from("api_key"), JsonValue::Text(key.to_owned()));
    }
    Deployment::from_params(params).expect("the deployment")
}

pub struct Env {
    pub global_api_base: Option<String>,
    pub global_api_key: Option<SecretString>,
    pub environment_api_base: Option<String>,
    pub environment_api_key: Option<SecretString>,
    pub headers: Option<Headers>,
    pub credentials: DeploymentSettings,
    pub environment: BTreeMap<String, String>,
}

impl Default for Env {
    fn default() -> Self {
        Self {
            global_api_base: None,
            global_api_key: None,
            environment_api_base: None,
            environment_api_key: None,
            headers: None,
            credentials: DeploymentSettings::default(),
            environment: BTreeMap::new(),
        }
    }
}

impl Env {
    pub fn with_globals(api_base: &str, api_key: &str) -> Self {
        Self {
            global_api_base: Some(api_base.to_owned()),
            global_api_key: Some(SecretString::from(api_key)),
            ..Self::default()
        }
    }

    pub fn globals(&self) -> Globals<'_> {
        Globals {
            api_base: self.global_api_base.as_deref(),
            api_key: self.global_api_key.as_ref(),
            environment_api_base: self.environment_api_base.as_deref(),
            environment_api_key: self.environment_api_key.as_ref(),
            headers: self.headers.as_ref(),
            environment: &self.environment,
            credentials: &self.credentials,
            refresh_enabled: false,
            sources: &NoSources,
            token_provider: None,
        }
    }
}
