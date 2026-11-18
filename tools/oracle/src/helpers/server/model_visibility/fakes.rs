use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::{AuthError, AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_server::model_visibility::healthy::RouterHealth;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use super::error::HealthError;

// The debug note the healthy filter writes when the router reports no unhealthy state.
const DEBUG_PREFIX: &str = "healthy-only model listing is enabled";

// B307's answer, injected so a case can make the cache refuse the read.
pub(crate) struct HealthReport {
    pub(crate) names: BTreeSet<String>,
    pub(crate) fails: bool,
    pub(crate) reads: Arc<AtomicUsize>,
}

#[async_trait]
impl RouterHealth for HealthReport {
    type Error = HealthError;

    async fn fully_unhealthy_model_names(&self) -> Result<BTreeSet<String>, HealthError> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        if self.fails {
            return Err(HealthError::Unreachable);
        }
        Ok(self.names.clone())
    }
}

// The caller the case asks as; the role is the only field either side reads.
pub(crate) fn user(role: Option<&str>) -> Result<UserAPIKeyAuth, AuthError> {
    let mut fields: BTreeMap<String, JsonData> = BTreeMap::new();
    fields.insert(
        "api_key".to_owned(),
        JsonData::String("sk-helper".to_owned()),
    );
    if let Some(role) = role {
        fields.insert("user_role".to_owned(), JsonData::String(role.to_owned()));
    }
    UserAPIKeyAuth::construct(AuthInput::Dict(fields))
}

// The healthy filter's one debug event, kept as the message it wrote.
#[derive(Clone, Default)]
pub(crate) struct DebugNotes(Arc<Mutex<Vec<String>>>);

#[derive(Default)]
struct RecordedNote(Option<String>);

impl Visit for RecordedNote {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = Some(format!("{value:?}"));
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for DebugNotes {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != Level::DEBUG {
            return;
        }
        let mut note = RecordedNote::default();
        event.record(&mut note);
        let Some(message) = note.0.filter(|text| text.starts_with(DEBUG_PREFIX)) else {
            return;
        };
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message);
    }
}

// The run's value and the debug notes it wrote while the subscriber was installed.
pub(crate) fn captured<T>(run: &dyn Fn() -> T) -> (T, Vec<String>) {
    let notes = DebugNotes::default();
    let subscriber = tracing_subscriber::registry().with(notes.clone());
    let value = with_default(subscriber, run);
    let written = notes
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    (value, written)
}
