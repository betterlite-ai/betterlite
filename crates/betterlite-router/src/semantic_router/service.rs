use std::error::Error;
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_models::decisions::Json;
use betterlite_openai::chat::ChatMessage;
use indexmap::IndexMap;
use tokio::sync::watch;
use tokio_util::task::TaskTracker;

use super::build::{BuildSlot, BuildState, build_entries, wait_for_build};
use super::error::RouterError;
use super::index::{RouteIndex, RouteIndexError, load_file, parse_inline};
use super::select::{BuiltEntry, select};
use super::text::{clamp, last_user_text};

pub const DEFAULT_MAX_INPUT_CHARS: i64 = 2000;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CallerContext {
    pub metadata: IndexMap<String, Json>,
    pub litellm_metadata: IndexMap<String, Json>,
    pub turn_off_message_logging: Option<bool>,
    pub litellm_session_id: Option<String>,
    pub litellm_trace_id: Option<String>,
}

pub trait RouteEmbedder: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    // one call per batch, cut to the character limit exactly as LiteLLMRouterEncoder cuts it
    fn embed_documents(
        &self,
        model: &str,
        texts: Vec<String>,
    ) -> impl Future<Output = Result<Vec<Vec<f64>>, Self::Error>> + Send;

    // the provider input is the clamped text, while the proxy request body keeps the whole text
    fn embed_query(
        &self,
        model: &str,
        text: String,
        raw_text: &str,
        caller: &CallerContext,
    ) -> impl Future<Output = Result<Vec<Vec<f64>>, Self::Error>> + Send;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoRouterSettings {
    pub config_path: Option<PathBuf>,
    pub config_json: Option<String>,
    pub default_model: String,
    pub embedding_model: String,
    pub max_input_chars: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteDecision {
    pub route: Option<String>,
    pub model: String,
}

pub struct AutoRouter<E>
where
    E: RouteEmbedder + 'static,
{
    index: Arc<RouteIndex>,
    settings: AutoRouterSettings,
    embedder: Arc<E>,
    // the caller's tracker, so the server drains the index build on shutdown
    tasks: TaskTracker,
    max_input_chars: i64,
    slot: Arc<Mutex<BuildSlot<E::Error>>>,
}

impl<E> AutoRouter<E>
where
    E: RouteEmbedder + 'static,
{
    pub fn new(
        settings: AutoRouterSettings,
        embedder: Arc<E>,
        tasks: TaskTracker,
    ) -> Result<Self, RouteIndexError> {
        // the source reads both settings for truth, so an empty string chooses the other source
        let path = settings
            .config_path
            .as_deref()
            .filter(|path| !path.as_os_str().is_empty());
        let config = settings
            .config_json
            .as_deref()
            .filter(|config| !config.is_empty());
        let index = match (path, config) {
            (Some(path), _) => load_file(path)?,
            (None, Some(config)) => parse_inline(config)?,
            (None, None) => return Err(RouteIndexError::MissingConfig),
        };
        let max_input_chars = settings.max_input_chars.unwrap_or(DEFAULT_MAX_INPUT_CHARS);
        Ok(Self {
            index: Arc::new(index),
            settings,
            embedder,
            tasks,
            max_input_chars,
            slot: Arc::new(Mutex::new(BuildSlot {
                built: None,
                waiters: None,
            })),
        })
    }

    // the route index builds before the caller is read, so a failed build reaches it even when its metadata is unusable
    pub async fn decide_with<C>(
        &self,
        messages: Option<&[ChatMessage]>,
        caller: C,
    ) -> Result<Option<RouteDecision>, Arc<RouterError<E::Error>>>
    where
        C: FnOnce() -> Option<CallerContext>,
    {
        let Some(messages) = messages else {
            return Ok(None);
        };
        let built = self.built_index().await?;
        let text = last_user_text(messages);
        let Some(caller) = caller() else {
            return Ok(Some(self.default_decision()));
        };
        Ok(Some(self.route(&built, &text, &caller).await))
    }

    #[must_use]
    pub fn default_decision(&self) -> RouteDecision {
        RouteDecision {
            route: None,
            model: self.settings.default_model.clone(),
        }
    }

    // a failed build reaches the caller, while a failed query only falls back to the default model
    async fn route(
        &self,
        built: &[BuiltEntry],
        text: &str,
        caller: &CallerContext,
    ) -> RouteDecision {
        let Some(query) = self.query_vector(text, caller).await else {
            return self.default_decision();
        };
        match select(built, &query) {
            Ok(Some(route)) => {
                let model = if route.is_empty() {
                    self.settings.default_model.clone()
                } else {
                    route.clone()
                };
                RouteDecision {
                    route: Some(route),
                    model,
                }
            }
            Ok(None) => self.default_decision(),
            Err(error) => {
                tracing::warn!(%error, "semantic routing failed, falling back to the default model");
                self.default_decision()
            }
        }
    }

    // any query-embedding failure warns and takes the default model; only a build failure reaches the caller
    async fn query_vector(&self, text: &str, caller: &CallerContext) -> Option<Vec<f64>> {
        let clamped = clamp(text, self.max_input_chars);
        let vectors = match self
            .embedder
            .embed_query(
                &self.settings.embedding_model,
                clamped.to_owned(),
                text,
                caller,
            )
            .await
        {
            Ok(vectors) => vectors,
            Err(error) => {
                tracing::warn!(%error, "semantic routing failed, falling back to the default model");
                return None;
            }
        };
        let Some(query) = vectors.into_iter().next() else {
            tracing::warn!("semantic routing failed: the query embedding call returned no vector");
            return None;
        };
        Some(query)
    }

    async fn built_index(&self) -> Result<Arc<Vec<BuiltEntry>>, Arc<RouterError<E::Error>>> {
        let receiver = {
            let mut slot = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(built) = &slot.built {
                return Ok(Arc::clone(built));
            }
            match slot.waiters.clone() {
                Some(waiters) => waiters,
                None => {
                    let (sender, receiver) = watch::channel(BuildState::Building);
                    slot.waiters = Some(receiver.clone());
                    self.spawn_build(sender);
                    receiver
                }
            }
        };
        wait_for_build(receiver).await
    }

    fn spawn_build(&self, sender: watch::Sender<BuildState<E::Error>>) {
        let index = Arc::clone(&self.index);
        let embedder = Arc::clone(&self.embedder);
        let slot = Arc::clone(&self.slot);
        let model = self.settings.embedding_model.clone();
        let limit = self.max_input_chars;
        let _build = self.tasks.spawn(async move {
            let outcome = build_entries(&index, embedder.as_ref(), &model, limit)
                .await
                .map_err(Arc::new);
            {
                let mut slot = slot.lock().unwrap_or_else(PoisonError::into_inner);
                if let Ok(entries) = &outcome {
                    slot.built = Some(Arc::clone(entries));
                }
                slot.waiters = None;
            }
            sender.send_replace(BuildState::Done(outcome));
        });
    }
}
