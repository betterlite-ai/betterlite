use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::sections::batch_provider_cancel::Settings;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::batch_provider_cancel::service::BatchProviderCancel;
use betterlite_core::api::batch_provider_cancel::{
    AzureCancelCall, BatchCancelDispatch, BatchProviderCancelError, BatchProviderCancelInput,
    BedrockCancelCall, OpenAiCancelCall, VertexCancelCall, XaiCancelCall,
};
use betterlite_openai::api::batch_records::BatchIdRequest;
use betterlite_openai::batches::records::BatchObject;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    input: BatchProviderCancelInput,
    settings: SettingsScript,
    env: BTreeMap<String, String>,
    now: i64,
    script: Script,
}

// the litellm_settings globals cancel_batch reads, scripted by the case
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct SettingsScript {
    api_base: Option<String>,
    api_version: Option<String>,
    api_key: Option<String>,
    openai_key: Option<String>,
    azure_key: Option<String>,
    organization: Option<String>,
    vertex_project: Option<String>,
    vertex_location: Option<String>,
}

impl SettingsScript {
    fn configured(&self, env: &BTreeMap<String, String>) -> Settings {
        Settings {
            api_base: self.api_base.clone(),
            api_version: self.api_version.clone(),
            api_key: self
                .api_key
                .as_ref()
                .map(|text| SecretString::from(text.clone())),
            openai_key: self
                .openai_key
                .as_ref()
                .map(|text| SecretString::from(text.clone())),
            azure_key: self
                .azure_key
                .as_ref()
                .map(|text| SecretString::from(text.clone())),
            azure_ad_token: env.get("AZURE_AD_TOKEN").cloned().map(SecretString::from),
            organization: self.organization.clone(),
            vertex_project: self.vertex_project.clone(),
            vertex_location: self.vertex_location.clone(),
            openai_base_url: env.get("OPENAI_BASE_URL").cloned(),
            openai_api_base: env.get("OPENAI_API_BASE").cloned(),
            openai_api_key: env.get("OPENAI_API_KEY").cloned().map(SecretString::from),
            openai_organization: env.get("OPENAI_ORGANIZATION").cloned(),
            azure_api_base: env.get("AZURE_API_BASE").cloned(),
            azure_api_version: env.get("AZURE_API_VERSION").cloned(),
            azure_openai_api_key: env
                .get("AZURE_OPENAI_API_KEY")
                .cloned()
                .map(SecretString::from),
            azure_api_key: env.get("AZURE_API_KEY").cloned().map(SecretString::from),
            vertexai_project: env.get("VERTEXAI_PROJECT").cloned(),
            vertexai_location: env.get("VERTEXAI_LOCATION").cloned(),
            vertexai_credentials: env
                .get("VERTEXAI_CREDENTIALS")
                .cloned()
                .map(SecretString::from),
        }
    }
}

// the provider answer the case scripts, or the failure it scripts instead
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Script {
    batch: Option<BatchObject>,
    error: Option<String>,
}

#[derive(Debug, Error)]
#[error("{message}")]
struct ScriptFailure {
    message: String,
}

struct Scripted {
    calls: Arc<Mutex<Vec<Value>>>,
    script: Script,
}

impl Scripted {
    fn record(&self, call: Value) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }

    fn reply(&self) -> Result<BatchObject, ScriptFailure> {
        if let Some(message) = &self.script.error {
            return Err(ScriptFailure {
                message: message.clone(),
            });
        }
        self.script.batch.clone().ok_or_else(|| ScriptFailure {
            message: "the case scripted no provider answer".to_owned(),
        })
    }
}

// the five provider branches differ only in their recorded fields, so one macro keeps them one shape
macro_rules! scripted_branches {
    ($(
        $method:ident($call_type:ident): $branch:literal => {
            $($key:literal: $value:expr),* $(,)?
        };
    )*) => {
        impl BatchCancelDispatch for Scripted {
            type Error = ScriptFailure;

            $(
                fn $method(
                    &self,
                    call: $call_type<'_>,
                ) -> impl Future<Output = Result<BatchObject, ScriptFailure>> + Send {
                    self.record(json!({
                        "branch": $branch,
                        $($key: $value,)*
                    }));
                    let reply = self.reply();
                    async move { reply }
                }
            )*
        }
    };
}

scripted_branches! {
    openai(OpenAiCancelCall): "openai" => {
        "is_async": call.is_async,
        "request": recorded_request(call.request),
        "api_base": call.api_base,
        "api_key": call.api_key.map(|key| key.expose_secret()),
        "organization": call.organization,
        "timeout": call.timeout,
        "max_retries": call.max_retries,
    };
    azure(AzureCancelCall): "azure" => {
        "is_async": call.is_async,
        "request": recorded_request(call.request),
        "api_base": call.api_base,
        "api_key": call.api_key.map(|key| key.expose_secret()),
        "api_version": call.api_version,
        "timeout": call.timeout,
        "max_retries": call.max_retries,
    };
    vertex(VertexCancelCall): "vertex" => {
        "is_async": call.is_async,
        "batch_id": call.batch_id,
        "api_base": call.api_base,
        "vertex_project": call.vertex_project,
        "vertex_location": call.vertex_location,
        "vertex_credentials": call.vertex_credentials.map(|token| token.expose_secret()),
        "timeout": call.timeout,
        "max_retries": call.max_retries,
    };
    xai(XaiCancelCall): "xai" => {
        "is_async": call.is_async,
        "batch_id": call.batch_id,
        "api_base": call.api_base,
        "api_key": call.api_key.map(|key| key.expose_secret()),
        "timeout": call.timeout,
    };
    bedrock(BedrockCancelCall): "bedrock" => {
        "batch_id": call.batch_id,
        "params": call.params,
    };
}

fn helper_error(
    error: &BatchProviderCancelError<ScriptFailure>,
) -> Box<crate::helpers::HelperError> {
    let (class, status) = match error.error_fields() {
        Some(fields) => ("BadRequestError", Some(fields.status)),
        None => ("ProviderError", None),
    };
    Box::new(crate::helpers::HelperError {
        class: class.to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let now = input.now;
        let service = BatchProviderCancel {
            clock: Arc::new(move || now),
            dispatch: Scripted {
                calls: Arc::clone(&calls),
                script: input.script.clone(),
            },
            limits: ApiLimits::default(),
            settings: input.settings.configured(&input.env),
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(|error| {
                Box::new(crate::helpers::HelperError {
                    class: "RuntimeError".to_owned(),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: error.to_string(),
                })
            })?;
        let result = runtime.block_on(service.run(&input.input));
        let calls = calls.lock().unwrap_or_else(PoisonError::into_inner).clone();
        match result {
            Ok(result) => Ok(json!({
                "batch": result.batch,
                "model": result.model,
                "calls": calls,
            })),
            Err(error) => Err(helper_error(&error)),
        }
    }
}
