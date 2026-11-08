use std::collections::{BTreeMap, VecDeque};
use std::future::ready;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;

use crate::argilla_logging::extraction::{ArgillaItem, CallbackKwargs, StandardLoggingPayload};
use crate::argilla_logging::hooks::{DatasetHook, DatasetHookError};
use crate::argilla_logging::logger::{ArgillaLogger, RandomSource};
use crate::argilla_logging::settings::{ArgillaSettings, ConstructorArguments, PayloadField};
use crate::argilla_logging::transport::{
    ArgillaMethod, ArgillaRequest, ArgillaResponse, Transport,
};

pub(crate) const DATASET_BODY: &str = r#"{"items": [{"id": "ds-1"}]}"#;

#[derive(Clone, Debug)]
pub(crate) struct Reply {
    pub status: u16,
    pub body: String,
}

impl Reply {
    pub(crate) fn ok(body: &str) -> Self {
        Self {
            status: 200,
            body: body.to_owned(),
        }
    }

    pub(crate) fn status(status: u16) -> Self {
        Self {
            status,
            body: String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Call {
    pub method: ArgillaMethod,
    pub url: String,
    pub api_key: String,
    pub body: Option<Vec<u8>>,
    pub timeout: Duration,
}

#[derive(Clone)]
pub(crate) struct ScriptedTransport {
    pub replies: Arc<Mutex<VecDeque<Reply>>>,
    pub calls: Arc<Mutex<Vec<Call>>>,
    pub fail: Arc<Mutex<bool>>,
}

impl ScriptedTransport {
    pub(crate) fn answering(replies: &[Reply]) -> Self {
        Self {
            replies: Arc::new(Mutex::new(replies.iter().cloned().collect())),
            calls: Arc::new(Mutex::new(Vec::new())),
            fail: Arc::new(Mutex::new(false)),
        }
    }

    pub(crate) fn calls(&self) -> Vec<Call> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn bodies(&self) -> Vec<Json> {
        let mut bodies = Vec::new();
        for call in self.calls() {
            let Some(body) = call.body.as_deref() else {
                continue;
            };
            if let Ok(parsed) = serde_json::from_slice(body) {
                bodies.push(parsed);
            }
        }
        bodies
    }

    pub(crate) fn start_failing(&self) {
        *self.fail.lock().unwrap_or_else(PoisonError::into_inner) = true;
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the scripted transport fails every call")]
pub(crate) struct ScriptedError;

impl Transport for ScriptedTransport {
    type Error = ScriptedError;

    async fn send(&self, request: ArgillaRequest) -> Result<ArgillaResponse, Self::Error> {
        ready(()).await;
        let ArgillaRequest {
            method,
            url,
            api_key,
            body,
            timeout,
        } = request;
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Call {
                method,
                url,
                api_key: api_key.expose_secret().to_owned(),
                body,
                timeout,
            });
        if *self.fail.lock().unwrap_or_else(PoisonError::into_inner) {
            return Err(ScriptedError);
        }
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| Reply::status(200));
        Ok(ArgillaResponse {
            status: reply.status,
            body: reply.body,
        })
    }
}

pub(crate) struct ScriptedRandom {
    pub values: Mutex<VecDeque<f64>>,
}

impl ScriptedRandom {
    pub(crate) fn new(values: &[f64]) -> Self {
        Self {
            values: Mutex::new(values.iter().copied().collect()),
        }
    }
}

impl RandomSource for ScriptedRandom {
    fn sample(&self) -> f64 {
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(1.0)
    }
}

#[derive(Clone, Copy)]
pub(crate) enum HookEffect {
    Pass,
    Filter,
    NotImplemented,
    Fail,
}

pub(crate) struct RecordingHook {
    pub effect: HookEffect,
    pub seen: Mutex<Vec<ArgillaItem>>,
}

impl RecordingHook {
    pub(crate) fn new(effect: HookEffect) -> Self {
        Self {
            effect,
            seen: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn seen(&self) -> Vec<ArgillaItem> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl DatasetHook for RecordingHook {
    async fn async_dataset_hook(
        &self,
        item: ArgillaItem,
        _payload: Option<&StandardLoggingPayload>,
    ) -> Result<Option<ArgillaItem>, DatasetHookError> {
        ready(()).await;
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(item.clone());
        match self.effect {
            HookEffect::Pass => Ok(Some(item)),
            HookEffect::Filter => Ok(None),
            HookEffect::NotImplemented => Err(DatasetHookError::NotImplemented),
            HookEffect::Fail => Err(DatasetHookError::Failed),
        }
    }
}

pub(crate) fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn transformation(fields: &[(&str, PayloadField)]) -> IndexMap<String, PayloadField> {
    fields
        .iter()
        .map(|(name, field)| ((*name).to_owned(), *field))
        .collect()
}

pub(crate) fn transformation_json() -> Json {
    Json::Object(IndexMap::from([(
        "input".to_owned(),
        Json::Text("messages".to_owned()),
    )]))
}

pub(crate) fn messages_json() -> Json {
    Json::List(vec![Json::Object(IndexMap::from([
        ("role".to_owned(), Json::Text("user".to_owned())),
        ("content".to_owned(), Json::Text("Hi".to_owned())),
    ]))])
}

pub(crate) fn response_json() -> Json {
    Json::Object(IndexMap::from([(
        "choices".to_owned(),
        Json::List(vec![Json::Object(IndexMap::from([(
            "message".to_owned(),
            Json::Object(IndexMap::from([(
                "content".to_owned(),
                Json::Text("Hello".to_owned()),
            )])),
        )]))]),
    )]))
}

pub(crate) fn payload(messages: Json, response: Json) -> CallbackKwargs {
    CallbackKwargs {
        standard_logging_object: Some(StandardLoggingPayload {
            messages: Some(messages),
            response: Some(response),
        }),
    }
}

pub(crate) fn valid_kwargs() -> CallbackKwargs {
    payload(messages_json(), response_json())
}

pub(crate) fn settings(pairs: &[(&str, &str)]) -> ArgillaSettings {
    let mut with_key = vec![("ARGILLA_API_KEY", "argilla-key")];
    with_key.extend_from_slice(pairs);
    ArgillaSettings::from_environment(
        &environment(&with_key),
        &ConstructorArguments::default(),
        Some(&transformation_json()),
    )
    .expect("the settings are valid")
}

pub(crate) async fn logger(
    pairs: &[(&str, &str)],
    replies: &[Reply],
    random: ScriptedRandom,
) -> ArgillaLogger<ScriptedTransport, ScriptedRandom> {
    let mut with_key = vec![("ARGILLA_API_KEY", "argilla-key")];
    with_key.extend_from_slice(pairs);
    let transport = ScriptedTransport::answering(replies);
    ArgillaLogger::connect(settings(&with_key), transport, random)
        .await
        .expect("the logger connects")
}
