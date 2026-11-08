#![expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "a LiteLLM completion request carries parameters the mirror forwards unchanged, and each target runs on the shadow thread LiteLLM itself starts"
)]

use std::fmt::Display;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::thread::JoinHandle as ThreadJoinHandle;
use std::thread::spawn as spawn_thread;

use serde_json::{Map, Value};
use tokio::runtime::Handle;
use tokio::task::JoinHandle as TaskJoinHandle;

mod tests;

const SPAN_FIELD: &str = "litellm_parent_otel_span";
const SILENT_FLAG: &str = "is_silent_experiment";
const LOGGING_KEYS: [&str; 3] = [
    "litellm_call_id",
    "litellm_logging_obj",
    "standard_logging_object",
];

#[derive(Debug, thiserror::Error)]
pub enum SilentExperimentError {
    #[error("metadata must be a JSON object")]
    MetadataNotObject,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirrorArgs<M> {
    pub model: String,
    pub messages: Vec<M>,
    pub stream: bool,
    pub request: Map<String, Value>,
}

// an implementor calls acompletion with the target, the messages, the stream flag and the request's other keys as arguments, reads a streamed answer to its end and gives up its chunks
pub trait MirrorCall<M: Clone + Send + 'static> {
    type Error: Display + Send + 'static;

    fn complete(
        &self,
        args: MirrorArgs<M>,
    ) -> Pin<Box<dyn Future<Output = Result<(), Self::Error>> + Send + 'static>>;
}

#[derive(Clone)]
pub struct SilentExperiments {
    pub runtime: Handle,
}

pub struct MirrorInvocation<C, M> {
    pub call: Arc<C>,
    pub target: String,
    pub messages: Vec<M>,
    pub request: Map<String, Value>,
}

impl SilentExperiments {
    #[must_use]
    // ast-grep-ignore: no-one-line-functions
    pub fn start<C, M>(&self, invocation: MirrorInvocation<C, M>) -> TaskJoinHandle<()>
    where
        C: MirrorCall<M> + Send + Sync + 'static,
        M: Clone + Send + 'static,
    {
        self.runtime.spawn(mirror(invocation))
    }

    #[must_use]
    // ast-grep-ignore: no-padded-one-line-functions
    pub fn start_blocking<C, M>(&self, invocation: MirrorInvocation<C, M>) -> ThreadJoinHandle<()>
    where
        C: MirrorCall<M> + Send + Sync + 'static,
        M: Clone + Send + 'static,
    {
        let runtime = self.runtime.clone();
        spawn_thread(move || runtime.block_on(mirror(invocation)))
    }
}

#[must_use]
pub fn silent_targets(value: Option<&Value>) -> Vec<String> {
    let Some(found) = value else {
        return Vec::new();
    };
    match found {
        Value::Null => Vec::new(),
        Value::String(target) => vec![target.clone()],
        Value::Array(items) => {
            let mut targets = Vec::with_capacity(items.len());
            for item in items {
                let Value::String(target) = item else {
                    return skip_shadow_traffic(found);
                };
                targets.push(target.clone());
            }
            targets
        }
        Value::Bool(_) | Value::Number(_) | Value::Object(_) => skip_shadow_traffic(found),
    }
}

pub fn silent_kwargs(
    request: &Map<String, Value>,
) -> Result<Map<String, Value>, SilentExperimentError> {
    let mut silent = request.clone();
    let metadata = silent
        .entry("metadata")
        .or_insert_with(|| Value::Object(Map::new()));
    if metadata.is_null() {
        *metadata = Value::Object(Map::new());
    }
    let Value::Object(fields) = metadata else {
        return Err(SilentExperimentError::MetadataNotObject);
    };
    fields.remove(SPAN_FIELD);
    fields.insert(SILENT_FLAG.to_owned(), Value::Bool(true));
    for key in LOGGING_KEYS {
        silent.remove(key);
    }
    Ok(silent)
}

fn already_a_silent_experiment(request: &Map<String, Value>) -> bool {
    request
        .get("metadata")
        .and_then(Value::as_object)
        .and_then(|metadata| metadata.get(SILENT_FLAG))
        .is_some_and(|flag| *flag == Value::Bool(true))
}

fn skip_shadow_traffic(value: &Value) -> Vec<String> {
    tracing::warn!(
        "silent_model must be a model name or a list of model names, got {value}; skipping shadow traffic"
    );
    Vec::new()
}

async fn mirror<C, M>(invocation: MirrorInvocation<C, M>)
where
    C: MirrorCall<M> + Send + Sync + 'static,
    M: Clone + Send + 'static,
{
    let MirrorInvocation {
        call,
        target,
        messages,
        request,
    } = invocation;
    if already_a_silent_experiment(&request) {
        return;
    }
    tracing::info!("Starting silent experiment for model {target}");
    let mut silent_request = match silent_kwargs(&request) {
        Ok(silent_request) => silent_request,
        Err(error) => {
            tracing::error!("Silent experiment failed for model {target}: {error}");
            return;
        }
    };
    if let Some(fields) = silent_request
        .get_mut("metadata")
        .and_then(Value::as_object_mut)
    {
        fields.insert("model_group".to_owned(), Value::String(target.clone()));
    }
    let stream = silent_request
        .remove("stream")
        .is_some_and(|flag| flag == Value::Bool(true));
    let args = MirrorArgs {
        model: target.clone(),
        messages,
        stream,
        request: silent_request,
    };
    if let Err(error) = call.complete(args).await {
        tracing::error!("Silent experiment failed for model {target}: {error}");
    }
}
