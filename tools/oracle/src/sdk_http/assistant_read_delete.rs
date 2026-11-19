use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::{
    azure_endpoints::Settings as AzureSettings, resource_sdk_settings::Settings,
};
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::assistant_read_delete::error::AssistantReadDeleteError;
use betterlite_core::api::assistant_read_delete::request::{
    AssistantReadDeleteInput, AssistantWrapper,
};
use betterlite_core::api::assistant_read_delete::sdk_dispatch::{
    AssistantSdkError, NoSources, SdkDispatch,
};
use betterlite_core::api::assistant_read_delete::{AssistantReadDelete, AssistantStep};
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_openai::assistants::error::AssistantsError;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::native::failure as mapped_failure;
use super::result::kinds::SdkResult;
use super::vector_query_embedding::outbound_client;

// the case names the harness mock in place of any configured base
const API_BASE_SENTINEL: &str = "$API_BASE";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Globals {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub openai_key: Option<SecretString>,
    pub azure_key: Option<SecretString>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntrySet {
    #[default]
    Both,
    Sync,
    Async,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    #[serde(default)]
    pub globals: Globals,
    #[serde(default)]
    pub entries: EntrySet,
    pub input: AssistantReadDeleteInput,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub runs: Vec<Run>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub runs: Vec<RunOutcome>,
}

// what one entry point returned or raised, projected as print() would show the caller
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct View(pub Value);
impl Eq for View {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunOutcome {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync: Option<View>,
    #[serde(rename = "async", skip_serializing_if = "Option::is_none")]
    pub asynchronous: Option<View>,
}

impl CallKind for Call {
    async fn send(
        &self,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> Result<SdkResult, SdkHttpError> {
        let transport = Arc::new(outbound_client(env)?);
        let mut runs = Vec::with_capacity(self.runs.len());
        for run in &self.runs {
            let (settings, azure) = resolve_settings(env, &run.globals, api_base)?;
            let harness = Harness {
                transport: Arc::clone(&transport),
                env,
                azure: &azure,
                settings: &settings,
            };
            let input = with_harness_base(&run.input, api_base);
            let sync = if run.entries == EntrySet::Async {
                None
            } else {
                Some(harness.execute(&input, AssistantWrapper::Sync).await)
            };
            let asynchronous = if run.entries == EntrySet::Sync {
                None
            } else {
                Some(harness.execute(&input, AssistantWrapper::Async).await)
            };
            runs.push(RunOutcome { sync, asynchronous });
        }
        Ok(SdkResult::AssistantReadDelete {
            value: Box::new(Outcome { runs }),
        })
    }
}

struct Harness<'a> {
    transport: Arc<ProxiedClient>,
    env: &'a BTreeMap<String, String>,
    azure: &'a AzureSettings,
    settings: &'a Settings,
}

impl Harness<'_> {
    async fn execute(&self, input: &AssistantReadDeleteInput, wrapper: AssistantWrapper) -> View {
        let mut input = input.clone();
        input.entry = wrapper;
        let facade = AssistantReadDelete {
            dispatch: SdkDispatch {
                transport: Arc::clone(&self.transport),
                sources: NoSources,
                environment: self.env.clone(),
                azure: self.azure.clone(),
            },
            limits: ApiLimits::default(),
            settings: self.settings.clone(),
        };
        match facade.run(&input).await {
            Ok(result) => step_view(result.step),
            Err(error) => failure_view(&error, wrapper),
        }
    }
}

fn step_view(step: AssistantStep) -> View {
    match step {
        AssistantStep::Page(page) => View(compact(json!({
            "page": {
                "object": page.object,
                "data": page
                    .data
                    .into_iter()
                    .map(|assistant| json!({
                        "id": assistant.id,
                        "object": assistant.object,
                        "created_at": assistant.created_at,
                        "model": assistant.model,
                        "name": assistant.name,
                    }))
                    .collect::<Vec<Value>>(),
                "has_more": page.has_more.unwrap_or_default(),
                "first_id": page.first_id,
                "last_id": page.last_id,
            }
        }))),
        AssistantStep::Deleted(deleted) => View(compact(json!({
            "deleted": { "id": deleted.id, "object": deleted.object, "deleted": deleted.deleted }
        }))),
        AssistantStep::Coroutine => View(compact(json!({ "coroutine": true }))),
    }
}

// litellm's own classes print as litellm.<Class>: message; a plain Exception prints only its message
fn failure_view(
    error: &AssistantReadDeleteError<AssistantSdkError>,
    wrapper: AssistantWrapper,
) -> View {
    let value = if let Some(fields) = error.fields() {
        let text = error.to_string();
        let message = if fields.prefixed {
            format!("litellm.{}: {text}", fields.class)
        } else {
            text
        };
        json!({
            "class": fields.class, "status": fields.status, "type": fields.error_type,
            "message": message,
        })
    } else if let AssistantReadDeleteError::Dispatch(AssistantSdkError::Assistants(
        AssistantsError::Status { status, body },
    )) = error
    {
        status_failure(*status, body, wrapper)
    } else {
        json!({ "class": "Exception", "message": error.to_string() })
    };
    View(compact(json!({ "failure": value })))
}

// the sync SDK names the status and prints the raw body; the async wrapper maps the body through B011
fn status_failure(status: u16, body: &[u8], wrapper: AssistantWrapper) -> Value {
    let text = String::from_utf8_lossy(body);
    let response = UpstreamResponse {
        status,
        headers: Default::default(),
        body: body.to_vec(),
    };
    let Ok(failure) = mapped_failure("", &response) else {
        return json!({ "class": "APIError", "status": status, "message": text });
    };
    if wrapper == AssistantWrapper::Sync {
        return json!({
            "class": sdk_class(status), "status": status, "type": failure.kind.flatten(),
            "param": failure.param.flatten(), "code": failure.code.flatten(),
            "message": format!("Error code: {status} - {text}"),
        });
    }
    // LiteLLM's mapped exception is built with body=None, so it carries no type, param or code
    json!({ "class": failure.class, "status": failure.status, "message": failure.message })
}

// the OpenAI SDK names a status failure after its status code
const fn sdk_class(status: u16) -> &'static str {
    match status {
        400 => "BadRequestError",
        401 => "AuthenticationError",
        403 => "PermissionDeniedError",
        404 => "NotFoundError",
        409 => "ConflictError",
        422 => "UnprocessableEntityError",
        429 => "RateLimitError",
        500..=599 => "InternalServerError",
        _ => "APIStatusError",
    }
}

// litellm's model_dump_json(exclude_none=True) leaves every null field out
fn compact(value: Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .into_iter()
                .filter(|(_, item)| !item.is_null())
                .map(|(key, item)| (key, compact(item)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(compact).collect()),
        other => other,
    }
}

fn resolve_settings(
    env: &BTreeMap<String, String>,
    globals: &Globals,
    api_base: &str,
) -> Result<(Settings, AzureSettings), SdkHttpError> {
    let loader = ConfigLoader {
        lookup: env.clone(),
    };
    let config = loader
        .load("model_list: []")
        .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    let mut settings = config.sections.resource_sdk_settings;
    settings.api_base = globals.api_base.clone().or(settings.api_base);
    settings.api_key = globals.api_key.clone().or(settings.api_key);
    settings.openai_key = globals.openai_key.clone().or(settings.openai_key);
    settings.azure_key = globals.azure_key.clone().or(settings.azure_key);
    harness_base(&mut settings.api_base, api_base);
    harness_base(&mut settings.openai_base_url, api_base);
    harness_base(&mut settings.openai_api_base, api_base);
    harness_base(&mut settings.azure_api_base, api_base);
    Ok((settings, config.sections.azure_endpoints))
}

// the harness owns the mock's base, so it takes the place of the sentinel in every base field
fn harness_base(value: &mut Option<String>, api_base: &str) {
    if value.as_deref() == Some(API_BASE_SENTINEL) {
        *value = Some(api_base.to_owned());
    }
}

fn with_harness_base(input: &AssistantReadDeleteInput, api_base: &str) -> AssistantReadDeleteInput {
    let mut input = input.clone();
    harness_base(&mut input.params.api_base, api_base);
    if let Some(client) = input.client.as_mut() {
        harness_base(&mut client.api_base, api_base);
    }
    input
}
