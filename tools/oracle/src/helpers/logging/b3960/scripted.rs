use std::collections::VecDeque;
use std::future::ready;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_logging::argilla_logging::extraction::{ArgillaItem, StandardLoggingPayload};
use betterlite_logging::argilla_logging::hooks::{DatasetHook, DatasetHookError};
use betterlite_logging::argilla_logging::logger::RandomSource;
use betterlite_logging::argilla_logging::transport::{
    ArgillaMethod, ArgillaRequest, ArgillaResponse, Transport,
};
use betterlite_logging::http_analytics_loggers::delivery::DELIVERY_DEADLINE;
use secrecy::ExposeSecret as _;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) body: String,
    #[serde(default)]
    pub(crate) raise: Option<String>,
}

#[derive(Clone)]
pub(crate) struct ScriptedTransport {
    pub(crate) replies: Arc<Mutex<VecDeque<Reply>>>,
    pub(crate) calls: Arc<Mutex<Vec<Value>>>,
}

#[derive(Debug, thiserror::Error)]
#[error("the scripted Argilla transport failed: {reason}")]
pub(crate) struct ScriptedError {
    pub(crate) reason: String,
}

fn call_json(request: &ArgillaRequest) -> Value {
    // LiteLLM passes no timeout for the module-level GET and POST, so the client default applies; the helper reports that deadline as null.
    let timeout = if request.timeout == DELIVERY_DEADLINE {
        Value::Null
    } else {
        json!(request.timeout.as_secs())
    };
    json!({
        "method": match request.method {
            ArgillaMethod::Get => "GET",
            ArgillaMethod::Post => "POST",
            ArgillaMethod::Put => "PUT",
        },
        "url": request.url.as_str(),
        "headers": { "x-argilla-api-key": request.api_key.expose_secret() },
        "body": request
            .body
            .as_deref()
            .map(|bytes| serde_json::from_slice(bytes).unwrap_or(Value::Null)),
        "timeout": timeout,
    })
}

impl Transport for ScriptedTransport {
    type Error = ScriptedError;

    async fn send(&self, request: ArgillaRequest) -> Result<ArgillaResponse, Self::Error> {
        ready(()).await;
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call_json(&request));
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        let Some(answer) = reply else {
            return Err(ScriptedError {
                reason: "no scripted reply".to_owned(),
            });
        };
        if let Some(reason) = answer.raise {
            return Err(ScriptedError { reason });
        }
        Ok(ArgillaResponse {
            status: answer.status,
            body: answer.body,
        })
    }
}

pub(crate) struct ScriptedRandom {
    pub(crate) values: Mutex<VecDeque<f64>>,
}

impl RandomSource for ScriptedRandom {
    fn sample(&self) -> f64 {
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        values.pop_front().unwrap_or(1.0)
    }
}

#[derive(Clone, Copy, Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HookEffect {
    Identity,
    Filter,
    NotImplemented,
    Failed,
}

pub(crate) struct ScriptedHook {
    pub(crate) effect: HookEffect,
    pub(crate) seen: Mutex<Vec<ArgillaItem>>,
    pub(crate) payload_present: AtomicBool,
}

#[async_trait::async_trait]
impl DatasetHook for ScriptedHook {
    async fn async_dataset_hook(
        &self,
        item: ArgillaItem,
        payload: Option<&StandardLoggingPayload>,
    ) -> Result<Option<ArgillaItem>, DatasetHookError> {
        ready(()).await;
        self.payload_present
            .store(payload.is_some(), Ordering::SeqCst);
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(item.clone());
        match self.effect {
            HookEffect::Identity => Ok(Some(item)),
            HookEffect::Filter => Ok(None),
            HookEffect::NotImplemented => Err(DatasetHookError::NotImplemented),
            HookEffect::Failed => Err(DatasetHookError::Failed),
        }
    }
}
