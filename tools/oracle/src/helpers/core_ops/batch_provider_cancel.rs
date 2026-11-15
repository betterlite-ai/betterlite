use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;

use betterlite_config::sections::batch_provider_cancel::Settings;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::batch_provider_cancel::service::BatchProviderCancel;
use betterlite_core::api::batch_provider_cancel::{
    BatchCancelDispatch, BatchProviderCancelError, BatchProviderCancelInput, CancelTarget,
};
use betterlite_openai::batches::records::BatchObject;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CaseInput {
    input: BatchProviderCancelInput,
    // the case scripts no litellm_settings globals when it names none
    settings: Option<SettingsScript>,
    env: BTreeMap<String, String>,
    now: i64,
    script: Script,
}

// the litellm_settings globals cancel_batch reads, scripted by the case
#[derive(Debug, Clone, Default, Deserialize)]
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
    script: Script,
}

impl Scripted {
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

// the pinned helper answers with no observed provider call, so the branch's target stays unrecorded
impl BatchCancelDispatch for Scripted {
    type Error = ScriptFailure;

    fn cancel(
        &self,
        _target: CancelTarget<'_>,
    ) -> impl Future<Output = Result<BatchObject, ScriptFailure>> + Send {
        let reply = self.reply();
        async move { reply }
    }
}

fn helper_error(
    error: &BatchProviderCancelError<ScriptFailure>,
) -> Box<crate::helpers::HelperError> {
    let (class, status) = error
        .error_fields()
        .map_or(("ProviderError", None), |fields| {
            ("BadRequestError", Some(fields.status))
        });
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
    type Input = CaseInput;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let now = input.now;
        let service = BatchProviderCancel {
            clock: Arc::new(move || now),
            dispatch: Scripted {
                script: input.script.clone(),
            },
            limits: ApiLimits::default(),
            settings: input
                .settings
                .clone()
                .unwrap_or_default()
                .configured(&input.env),
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
        match result {
            Ok(answer) => Ok(json!({
                "batch": answer.batch,
                "model": answer.model,
                "calls": [],
            })),
            Err(error) => Err(helper_error(&error)),
        }
    }
}
