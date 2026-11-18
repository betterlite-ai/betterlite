use std::error::Error;
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_models::decisions::Json;
use betterlite_openai::chat::ChatMessage;
use indexmap::IndexMap;
use tokio::sync::watch;
use tokio::time::timeout;
use tokio_util::task::TaskTracker;

use super::build::{BuildSlot, BuildState, build_entries, wait_for_build};
use super::error::RouterError;
use super::index::{RouteIndex, RouteIndexError, load_file, parse_inline};
use super::select::{BuiltEntry, select};
use super::text::{clamp, last_user_text};

pub const DEFAULT_MAX_INPUT_CHARS: i64 = 2000;
// litellm.request_timeout's pinned default, the deadline every embedding call behind the encoder gets
pub const EMBEDDING_TIMEOUT: Duration = Duration::from_secs(6000);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
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

// what a caller does with the index build: read a finished one, join a running one, or start the first
enum BuildPlan<E>
where
    E: Error + Send + Sync + 'static,
{
    Built(Arc<[BuiltEntry]>),
    Wait(watch::Receiver<BuildState<E>>),
    Start(watch::Sender<BuildState<E>>, watch::Receiver<BuildState<E>>),
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
            (Some(file), _) => load_file(file)?,
            (None, Some(inline)) => parse_inline(inline)?,
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
        let Some(conversation) = messages else {
            return Ok(None);
        };
        let built = self.built_index().await?;
        let text = last_user_text(conversation);
        let Some(context) = caller() else {
            return Ok(Some(self.default_decision()));
        };
        Ok(Some(self.route(&built, &text, &context).await))
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
        let answer = timeout(
            EMBEDDING_TIMEOUT,
            self.embedder.embed_query(
                &self.settings.embedding_model,
                clamped.to_owned(),
                text,
                caller,
            ),
        )
        .await;
        let vectors = match answer {
            Ok(Ok(vectors)) => vectors,
            Ok(Err(error)) => {
                tracing::warn!(%error, "semantic routing failed, falling back to the default model");
                return None;
            }
            Err(_elapsed) => {
                tracing::warn!(
                    "semantic routing failed: the query embedding call timed out, falling back to the default model"
                );
                return None;
            }
        };
        let Some(query) = vectors.into_iter().next() else {
            tracing::warn!("semantic routing failed: the query embedding call returned no vector");
            return None;
        };
        Some(query)
    }

    async fn built_index(&self) -> Result<Arc<[BuiltEntry]>, Arc<RouterError<E::Error>>> {
        match self.plan_build() {
            BuildPlan::Built(built) => Ok(built),
            BuildPlan::Wait(receiver) => wait_for_build(receiver).await,
            BuildPlan::Start(sender, receiver) => {
                self.spawn_build(sender);
                wait_for_build(receiver).await
            }
        }
    }

    fn plan_build(&self) -> BuildPlan<E::Error> {
        let mut slot = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
        let done = slot.built.clone();
        let running = slot.waiters.clone();
        let plan = match (done, running) {
            (Some(built), _) => BuildPlan::Built(built),
            (None, Some(waiters)) => BuildPlan::Wait(waiters),
            (None, None) => {
                let (sender, receiver) = watch::channel(BuildState::Building);
                slot.waiters = Some(receiver.clone());
                BuildPlan::Start(sender, receiver)
            }
        };
        drop(slot);
        plan
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
            let mut held = slot.lock().unwrap_or_else(PoisonError::into_inner);
            if let Ok(entries) = &outcome {
                held.built = Some(Arc::clone(entries));
            }
            held.waiters = None;
            drop(held);
            sender.send_replace(BuildState::Done(outcome));
        });
    }
}
