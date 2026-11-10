use std::future::{Future, ready};
use std::io;
use std::sync::{Mutex, PoisonError};

use betterlite_models::fetch_policy::response::FetchResponse;
use betterlite_models::lemonade_model_metadata::{ModelInfoClient, ModelInfoRequest};
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_models::provider_metadata_factory::adapters::DynamicModelInfo;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::value::RawValue;
use serde_json::{Value, json};

use super::input::{DynamicScript, HfScript};

pub(crate) struct ScriptedDynamic {
    script: Option<DynamicScript>,
    calls: Mutex<Vec<Value>>,
}

impl ScriptedDynamic {
    pub(crate) fn new(script: Option<&DynamicScript>) -> Self {
        Self {
            script: script.cloned(),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn calls(&self) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl DynamicMetadataSource for ScriptedDynamic {
    type Error = io::Error;

    fn get_model_info(
        &self,
        adapter: DynamicModelInfo,
        model: &str,
        api_base: Option<&str>,
        api_key: Option<&SecretString>,
    ) -> impl Future<Output = Result<Option<Box<RawValue>>, io::Error>> + Send {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({
                "adapter": kind(adapter),
                "model": model,
                "api_base": api_base,
                "api_key": api_key.map(|key| key.expose_secret().to_owned()),
            }));
        let answer = match &self.script {
            Some(DynamicScript::Value { body }) => Ok(Some(body.clone())),
            Some(DynamicScript::Error) => Err(io::Error::other("dynamic model info failed")),
            Some(DynamicScript::None) | None => Ok(None),
        };
        ready(answer)
    }
}

const fn kind(adapter: DynamicModelInfo) -> &'static str {
    match adapter {
        DynamicModelInfo::Ollama => "ollama",
        DynamicModelInfo::Lemonade => "lemonade",
    }
}

pub(crate) struct ScriptedHf {
    script: Option<HfScript>,
    requests: Mutex<Vec<Value>>,
}

impl ScriptedHf {
    pub(crate) fn new(script: Option<&HfScript>) -> Self {
        Self {
            script: script.cloned(),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn requests(&self) -> Vec<Value> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ModelInfoClient for &ScriptedHf {
    type Error = io::Error;

    fn get(
        &self,
        request: &ModelInfoRequest,
    ) -> impl Future<Output = Result<FetchResponse, io::Error>> + Send {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({
                "url": request.url,
                "timeout_seconds": request.timeout.as_secs_f64(),
            }));
        let answer = match &self.script {
            Some(HfScript::Status { status, body }) => Ok(FetchResponse::new(
                *status,
                Vec::new(),
                body.as_bytes().to_vec(),
            )),
            Some(HfScript::Timeout) => Err(io::Error::new(io::ErrorKind::TimedOut, "timed out")),
            Some(HfScript::TransportError) | None => Err(io::Error::other("config fetch failed")),
        };
        ready(answer)
    }
}
