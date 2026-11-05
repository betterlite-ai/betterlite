use crate::helpers::{HelperError, HelperResult};
use betterlite_config::loader::ConfigLoader;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::vector_stores::files::{
    self, FileCreateParams, FileListParams, FileOperation, FileUpdateParams,
};
use betterlite_openai::vector_stores::reply::{ReplyKind, transform_response};
use betterlite_openai::vector_stores::store::{self, CreateParams, Query, SearchParams};
use betterlite_openai::vector_stores::{
    Headers, Request, complete_url, file_endpoints, get_auth_credentials, store_endpoints,
    validate_environment,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

mod tests;

type Env = BTreeMap<String, String>;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EnvironmentInput {
    #[serde(default)]
    headers: Env,
    api_key: Option<String>,
    litellm_api_key: Option<String>,
    litellm_openai_key: Option<String>,
    #[serde(default)]
    env: Env,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UrlInput {
    api_base: Option<String>,
    litellm_api_base: Option<String>,
    vector_store_id: String,
    #[serde(default)]
    env: Env,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SearchInput {
    vector_store_id: String,
    query: Query,
    #[serde(default)]
    params: SearchParams,
    api_base: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateInput {
    #[serde(default)]
    params: CreateParams,
    api_base: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileItemInput {
    operation: FileOperation,
    file_id: String,
    api_base: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileUpdateInput {
    file_id: String,
    params: FileUpdateParams,
    api_base: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyInput {
    kind: String,
    status_code: u16,
    #[serde(default)]
    headers: Env,
    body: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Surface {
    Store,
    Files,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub(crate) enum Call {
    AuthCredentials {
        api_key: Option<String>,
    },
    ValidateEnvironment(EnvironmentInput),
    CompleteUrl(UrlInput),
    Endpoints {
        surface: Surface,
    },
    SearchRequest(SearchInput),
    CreateRequest(CreateInput),
    FileCreateRequest {
        params: FileCreateParams,
        api_base: String,
    },
    FileListRequest {
        params: FileListParams,
        api_base: String,
    },
    FileItemRequest(FileItemInput),
    FileUpdateRequest(FileUpdateInput),
    Reply(ReplyInput),
}

fn failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn headers_value(headers: &Headers) -> Value {
    let mut exposed = BTreeMap::new();
    for (name, value) in headers {
        exposed.insert(name.as_str(), value.expose_secret());
    }
    json!(exposed)
}

fn environment(input: &EnvironmentInput) -> HelperResult {
    let config = ConfigLoader {
        lookup: input.env.clone(),
    }
    .load("")
    .map_err(|error| failure("InvalidInput", error.to_string()))?;
    let globals = LiteLlmGlobals {
        api_base: None,
        api_key: input.litellm_api_key.as_deref().map(SecretString::from),
        openai_key: input.litellm_openai_key.as_deref().map(SecretString::from),
        organization: None,
    };
    let env_key = config.sections.resource_sdk_settings.openai_api_key;
    let caller = input.headers.iter();
    let headers = caller
        .map(|(name, value)| (name.clone(), SecretString::from(value.as_str())))
        .collect();
    Ok(headers_value(&validate_environment(
        headers,
        input.api_key.as_deref().map(SecretString::from),
        globals,
        env_key,
    )))
}

fn urls(input: &UrlInput) -> HelperResult {
    let config = ConfigLoader {
        lookup: input.env.clone(),
    }
    .load("")
    .map_err(|error| failure("InvalidInput", error.to_string()))?;
    let settings = &config.sections.openai_credentials;
    let (base, global) = (input.api_base.as_deref(), input.litellm_api_base.as_deref());
    let id = Some(input.vector_store_id.as_str());
    let on_store = complete_url(base, global, settings, None);
    let on_files = complete_url(base, global, settings, id);
    match (on_store, on_files) {
        (Ok(store), Ok(files)) => Ok(json!({"store": store, "files": files})),
        (Err(error), _) | (_, Err(error)) => Err(failure("ValueError", error.to_string())),
    }
}

fn reply(input: &ReplyInput) -> HelperResult {
    let kind = match input.kind.as_str() {
        "search" => ReplyKind::Search,
        "create" => ReplyKind::Create,
        _ => ReplyKind::File,
    };
    let mut built = http::Response::builder().status(input.status_code);
    for (name, value) in &input.headers {
        built = built.header(name, value);
    }
    let response = built
        .body(input.body.clone().into_bytes())
        .map_err(|error| failure("InvalidInput", error.to_string()))?;
    let sent = response.headers().iter();
    let headers = sent.map(|(name, value)| (name.as_str(), value.to_str().unwrap_or_default()));
    let failed = json!({"status_code": response.status().as_u16(), "headers": headers.collect::<BTreeMap<_, _>>()});
    Ok(transform_response(kind, &response).map_or(failed, |decoded| json!(decoded)))
}

fn requested<E>(request: Result<Request, E>) -> HelperResult
where
    E: std::fmt::Display,
{
    request
        .map(|sent| json!(sent))
        .map_err(|error| failure("ValueError", error.to_string()))
}

fn endpoints(surface: &Surface) -> Value {
    match surface {
        Surface::Store => json!(store_endpoints()),
        Surface::Files => json!(file_endpoints()),
    }
}

fn invoke(input: &Call) -> HelperResult {
    match input {
        Call::AuthCredentials { api_key } => {
            get_auth_credentials(api_key.as_deref().map(SecretString::from).as_ref())
                .map(|headers| headers_value(&headers))
                .map_err(|error| failure("ValueError", error.to_string()))
        }
        Call::ValidateEnvironment(fields) => environment(fields),
        Call::CompleteUrl(fields) => urls(fields),
        Call::Endpoints { surface } => Ok(endpoints(surface)),
        Call::SearchRequest(fields) => requested(store::search_request(
            &fields.vector_store_id,
            &fields.query,
            &fields.params,
            &fields.api_base,
        )),
        Call::CreateRequest(fields) => Ok(json!(store::create_request(
            &fields.params,
            &fields.api_base
        ))),
        Call::FileCreateRequest { params, api_base } => {
            Ok(json!(files::create_request(params, api_base)))
        }
        Call::FileListRequest { params, api_base } => {
            Ok(json!(files::list_request(params, api_base)))
        }
        Call::FileItemRequest(fields) => requested(files::file_request(
            &fields.operation,
            &fields.file_id,
            &fields.api_base,
        )),
        Call::FileUpdateRequest(fields) => requested(files::file_request(
            &FileOperation::Update(fields.params.clone()),
            &fields.file_id,
            &fields.api_base,
        )),
        Call::Reply(fields) => reply(fields),
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Call;

    fn run(input: &Self::Input) -> HelperResult {
        invoke(input)
    }
}
