use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use secrecy::SecretString;
use tokio::runtime::Handle;
use tokio::task::JoinSet;

use crate::types::response::InteractionsApiResponse;

use super::context::{InteractionPollContext, SettlementFlag};
use super::host::{FetchInteraction, SettlementHost};
use super::poll::poll_background_cost;
use super::status::is_pollable_status;

pub struct BackgroundPollRegistry<Host> {
    tasks: JoinSet<()>,
    active: Arc<Mutex<HashMap<String, ActivePoll<Host>>>>,
    next_token: u64,
}

impl<Host> Default for BackgroundPollRegistry<Host> {
    fn default() -> Self {
        Self {
            tasks: JoinSet::new(),
            active: Arc::new(Mutex::new(HashMap::new())),
            next_token: 0,
        }
    }
}

pub struct BackgroundPollRequest<Host> {
    pub response: InteractionsApiResponse,
    pub host: Option<Host>,
    pub custom_llm_provider: String,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub polling_enabled: bool,
    pub initial_interval: Duration,
    pub max_interval: Duration,
    pub timeout: Duration,
}

struct ActivePoll<Host> {
    token: u64,
    context: InteractionPollContext<Host>,
}

struct ActivePollGuard<Host> {
    active: Arc<Mutex<HashMap<String, ActivePoll<Host>>>>,
    interaction_id: String,
    token: u64,
}

impl<Host> Drop for ActivePollGuard<Host> {
    fn drop(&mut self) {
        let mut active = self.active.lock().unwrap_or_else(PoisonError::into_inner);
        if active
            .get(&self.interaction_id)
            .is_some_and(|poll| poll.token == self.token)
        {
            active.remove(&self.interaction_id);
        }
    }
}

impl<Host> BackgroundPollRegistry<Host> {
    pub fn schedule<Error>(
        &mut self,
        context: InteractionPollContext<Host>,
        fetch: FetchInteraction<Host, Error>,
    ) -> bool
    where
        Host: SettlementHost + Clone + 'static,
        Error: fmt::Display + Send + 'static,
    {
        if Handle::try_current().is_err() {
            return false;
        }
        let token = self.next_token;
        self.next_token = self.next_token.wrapping_add(1);
        let interaction_id = context.interaction_id.clone();
        let guard = ActivePollGuard {
            active: Arc::clone(&self.active),
            interaction_id: interaction_id.clone(),
            token,
        };
        self.active
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                interaction_id,
                ActivePoll {
                    token,
                    context: context.clone(),
                },
            );
        self.tasks.spawn(async move {
            let _guard = guard;
            poll_background_cost(context, fetch).await;
        });
        true
    }

    #[must_use]
    pub fn active_poll(&self, interaction_id: &str) -> Option<InteractionPollContext<Host>>
    where
        Host: Clone,
    {
        self.active
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(interaction_id)
            .map(|poll| poll.context.clone())
    }

    pub async fn shutdown(&mut self) {
        self.tasks.shutdown().await;
        self.active
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }
}

pub fn maybe_schedule_poll<Host, Error>(
    registry: &mut BackgroundPollRegistry<Host>,
    request: BackgroundPollRequest<Host>,
    fetch: FetchInteraction<Host, Error>,
) -> bool
where
    Host: SettlementHost + Clone + 'static,
    Error: fmt::Display + Send + 'static,
{
    if !request.polling_enabled {
        return false;
    }
    let Some(interaction_id) = request
        .response
        .id
        .clone()
        .filter(|id| !id.is_empty() && is_pollable_status(request.response.status.as_deref()))
    else {
        return false;
    };
    let Some(host) = request.host else {
        return false;
    };
    let context = InteractionPollContext {
        interaction_id,
        custom_llm_provider: request.custom_llm_provider,
        host,
        api_key: request.api_key,
        api_base: request.api_base,
        initial_interval: request.initial_interval,
        max_interval: request.max_interval,
        timeout: request.timeout,
        settlement: SettlementFlag::default(),
    };
    registry.schedule(context, fetch)
}
