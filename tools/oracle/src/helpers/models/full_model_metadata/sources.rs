use std::future::{Future, ready};
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_models::fetch_policy::response::FetchResponse;
use betterlite_models::full_model_metadata::parameters::SupportedParameterSource;
use betterlite_models::lemonade_model_metadata::{ModelInfoClient, ModelInfoRequest};
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_models::provider_metadata_factory::adapters::DynamicModelInfo;
use secrecy::SecretString;
use serde_json::value::{RawValue, to_raw_value};
use serde_json::{Value, json};

use super::input::ParameterScript;

#[derive(Debug, Default)]
pub(crate) struct CallLog {
    entries: Mutex<Vec<Value>>,
}

impl CallLog {
    pub(crate) fn push(&self, entry: Value) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entry);
    }

    pub(crate) fn len(&self) -> usize {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    pub(crate) fn since(&self, start: usize) -> Vec<Value> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(start..)
            .unwrap_or_default()
            .to_vec()
    }

    fn parameters(&self, model: &str, provider: Option<&str>, answer: Option<&[String]>) {
        self.push(json!({
            "kind": "parameters",
            "model": model,
            "provider": provider,
            "answer": answer,
        }));
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the scripted parameter query failed")]
pub(crate) struct ParameterFailure;

pub(crate) struct ScriptedSource {
    pub(crate) script: ParameterScript,
    pub(crate) log: Arc<CallLog>,
}

impl SupportedParameterSource for ScriptedSource {
    type Error = ParameterFailure;

    fn supported_openai_params(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Result<Option<Vec<String>>, ParameterFailure> {
        match &self.script {
            ParameterScript::Value { body } => {
                self.log
                    .parameters(model, custom_llm_provider, body.as_deref());
                Ok(body.clone())
            }
            ParameterScript::Absent => {
                self.log.parameters(model, custom_llm_provider, None);
                Ok(None)
            }
            ParameterScript::Error => {
                self.log.push(json!({
                    "kind": "parameters",
                    "model": model,
                    "provider": custom_llm_provider,
                    "raised": true,
                }));
                Err(ParameterFailure)
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct RecordingDynamic {
    pub(crate) script: Option<Value>,
    pub(crate) log: Arc<CallLog>,
}

impl DynamicMetadataSource for RecordingDynamic {
    type Error = io::Error;

    fn get_model_info(
        &self,
        adapter: DynamicModelInfo,
        model: &str,
        api_base: Option<&str>,
        api_key: Option<&SecretString>,
    ) -> impl Future<Output = Result<Option<Box<RawValue>>, io::Error>> + Send {
        self.log.push(json!({
            "kind": "dynamic",
            "adapter": adapter_name(adapter),
            "model": model,
            "api_base": api_base,
            "authenticated": api_key.is_some(),
        }));
        let Some(body) = &self.script else {
            return ready(Ok(None));
        };
        ready(to_raw_value(body).map(Some).map_err(io::Error::other))
    }
}

const fn adapter_name(adapter: DynamicModelInfo) -> &'static str {
    match adapter {
        DynamicModelInfo::Ollama => "ollama",
        DynamicModelInfo::Lemonade => "lemonade",
    }
}

#[derive(Clone, Copy, Debug, thiserror::Error)]
#[error("no Hugging Face config transport is configured")]
pub(crate) struct NoConfig;

impl ModelInfoClient for NoConfig {
    type Error = Self;

    fn get(
        &self,
        _request: &ModelInfoRequest,
    ) -> impl Future<Output = Result<FetchResponse, Self>> + Send {
        ready(Err(Self))
    }
}
