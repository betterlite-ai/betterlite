use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::sections::Section as _;
use betterlite_config::sections::azure_endpoints::Settings as AzureSettings;
use betterlite_config::sections::resource_sdk_settings::{Raw, RawSettings};
use betterlite_core::api::assistant_messages::connection::MessageParams;
use betterlite_core::api::assistant_messages::options::MessageData;
use betterlite_core::api::assistant_messages::provider::MessageContext;
use betterlite_core::api::assistant_messages::service::{self, MessageRequest, Reply};
use betterlite_core::api::resource_sdk_settings::ResourceSdkSettings;
use betterlite_foundry::chat::sources::NoSources;
use betterlite_http::proxy::ProxiedClient;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use secrecy::SecretString;

use crate::sdk_http::error::SdkHttpError;
use crate::sdk_http::result::kinds::SdkResult;
use crate::sdk_http::vector_query_embedding::outbound_client;

use super::failure;
use super::{Call, Mode, Operation, Outcome, Run, RunOutcome, Settings};

// every run answers through the mock origin the harness owns, so api_base replaces the case's own
pub(super) async fn send(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let mut runs = Vec::with_capacity(call.runs.len());
    for run in &call.runs {
        runs.push(run_once(run, api_base, env).await?);
    }
    Ok(SdkResult::AssistantMessages {
        value: Box::new(Outcome { runs }),
    })
}

async fn run_once(
    run: &Run,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<RunOutcome, SdkHttpError> {
    let settings = resource_settings(&run.settings, env)?;
    let params = message_params(run.params.as_ref(), api_base)?;
    let data = message_data(run)?;
    let azure = AzureSettings::default();
    let sources = NoSources;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?;
    let context = MessageContext {
        settings: &settings,
        azure: &azure,
        environment: env,
        sources: &sources,
        transport: Arc::new(outbound_client(env)?),
        redactor: &redactor,
    };
    let request = MessageRequest {
        provider: &run.provider,
        thread_id: &run.thread_id,
        data: data.as_ref(),
        params: &params,
        drop_params: run.drop_params,
    };
    match run.mode {
        Mode::Sync => sync_reply(run.operation, &request, &context).await,
        Mode::Async => async_reply(run.operation, &request, &context).await,
    }
}

// add_message and get_messages hand the provider exception to the caller unchanged
async fn sync_reply(
    operation: Operation,
    request: &MessageRequest<'_>,
    context: &MessageContext<'_, ProxiedClient>,
) -> Result<RunOutcome, SdkHttpError> {
    let reply = match operation {
        Operation::Append => service::add_message(request, context).await,
        Operation::List => service::get_messages(request, context).await,
    };
    match reply {
        Ok(reply) => reply_value(reply),
        Err(error) => failure::sync(&error),
    }
}

// a_add_message and aget_messages map the failure through exception_type
async fn async_reply(
    operation: Operation,
    request: &MessageRequest<'_>,
    context: &MessageContext<'_, ProxiedClient>,
) -> Result<RunOutcome, SdkHttpError> {
    let reply = match operation {
        Operation::Append => service::a_add_message(request, context).await,
        Operation::List => service::aget_messages(request, context).await,
    };
    match reply {
        Ok(reply) => reply_value(reply),
        Err(failure) => failure::mapped(&failure),
    }
}

fn reply_value(reply: Reply) -> Result<RunOutcome, SdkHttpError> {
    let encoded = match reply {
        Reply::Message(message) => serde_json::to_value(&message),
        Reply::Page(page) => serde_json::to_value(&page),
    }
    .map_err(SdkHttpError::Serialize)?;
    Ok(RunOutcome {
        value: Some(encoded),
        failure: None,
    })
}

fn message_data(run: &Run) -> Result<Option<MessageData>, SdkHttpError> {
    if matches!(run.operation, Operation::List) {
        return Ok(None);
    }
    let (Some(role), Some(content)) = (run.role.as_ref(), run.content.as_ref()) else {
        return Err(SdkHttpError::Invalid(
            "an append run needs a role and a content".to_owned(),
        ));
    };
    Ok(Some(MessageData {
        role: role.clone(),
        content: content.clone(),
        attachments: run.attachments.clone(),
        metadata: run.metadata.clone(),
    }))
}

// the core reads the credentials and timeout it resolves; the wrapper keywords ride along unread
fn message_params(params: Option<&Json>, api_base: &str) -> Result<MessageParams, SdkHttpError> {
    let mut fields = match params {
        None => Object::new(),
        Some(Json::Object(fields)) => fields.clone(),
        Some(_) => {
            return Err(SdkHttpError::Invalid(
                "the message params are not a mapping".to_owned(),
            ));
        }
    };
    fields.insert("api_base".to_owned(), Json::Text(api_base.to_owned()));
    let encoded = serde_json::to_value(Json::Object(fields)).map_err(SdkHttpError::Serialize)?;
    serde_json::from_value(encoded)
        .map_err(|error| SdkHttpError::Invalid(format!("invalid message params: {error}")))
}

fn resource_settings(
    settings: &Settings,
    env: &BTreeMap<String, String>,
) -> Result<ResourceSdkSettings, SdkHttpError> {
    let raw = Raw {
        settings: RawSettings {
            api_key: settings.api_key.clone().map(SecretString::from),
            api_version: settings.api_version.clone(),
            ..RawSettings::default()
        },
    };
    let converted = raw
        .convert(env)
        .map_err(|error| SdkHttpError::Invalid(format!("invalid message settings: {error}")))?;
    Ok(ResourceSdkSettings::from(&converted))
}
