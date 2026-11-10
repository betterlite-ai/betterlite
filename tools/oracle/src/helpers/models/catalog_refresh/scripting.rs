use std::collections::VecDeque;
use std::future::{Future, ready};
use std::str::FromStr as _;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_models::fetch_policy::client::{
    CatalogClient, CatalogClientError, CatalogRequest, FetchErrorKind,
};
use betterlite_models::fetch_policy::response::FetchResponse;
use betterlite_models::fetch_policy::retry::{CatalogSleeper, JitterSource};
use rust_decimal::Decimal;

use super::input::ReplyInput;
use super::invalid;
use crate::helpers::HelperError;

#[derive(Clone, Debug, thiserror::Error)]
#[error("{detail}")]
pub(crate) struct ScriptedError {
    pub(crate) kind: FetchErrorKind,
    pub(crate) detail: String,
}

impl CatalogClientError for ScriptedError {
    fn kind(&self) -> FetchErrorKind {
        self.kind
    }
}

pub(crate) struct ScriptedClient {
    replies: Mutex<VecDeque<Result<FetchResponse, ScriptedError>>>,
    pub(crate) requests: AtomicUsize,
}

impl ScriptedClient {
    pub(crate) fn new(replies: &[ReplyInput]) -> Self {
        let prepared = replies.iter().map(scripted_reply).collect();
        Self {
            replies: Mutex::new(prepared),
            requests: AtomicUsize::new(0),
        }
    }

    fn next(&self) -> Result<FetchResponse, ScriptedError> {
        self.requests.fetch_add(1, Ordering::SeqCst);
        match self.replies.lock() {
            Ok(mut replies) => replies.pop_front(),
            Err(poisoned) => poisoned.into_inner().pop_front(),
        }
        .unwrap_or_else(|| {
            Err(ScriptedError {
                kind: FetchErrorKind::Transport,
                detail: "unscripted request".to_owned(),
            })
        })
    }
}

fn scripted_reply(reply: &ReplyInput) -> Result<FetchResponse, ScriptedError> {
    match reply.error.as_deref() {
        Some("invalid_url") => Err(ScriptedError {
            kind: FetchErrorKind::InvalidUrl,
            detail: "unsupported url".to_owned(),
        }),
        Some(_) => Err(ScriptedError {
            kind: FetchErrorKind::Transport,
            detail: "connection failed".to_owned(),
        }),
        None => Ok(FetchResponse::new(
            reply.status,
            reply.headers.clone().into_iter().collect(),
            reply.body.as_bytes().to_vec(),
        )),
    }
}

impl CatalogClient for ScriptedClient {
    type Error = ScriptedError;

    fn get(&self, _request: &CatalogRequest) -> Result<FetchResponse, Self::Error> {
        self.next()
    }

    fn get_async(
        &self,
        _request: &CatalogRequest,
    ) -> impl Future<Output = Result<FetchResponse, Self::Error>> + Send {
        ready(self.next())
    }
}

#[derive(Default)]
pub(crate) struct ScriptedSleeper {
    seconds: Mutex<Vec<Decimal>>,
}

impl ScriptedSleeper {
    pub(crate) fn recorded(&self) -> Vec<Decimal> {
        match self.seconds.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

impl CatalogSleeper for ScriptedSleeper {
    fn sleep(&self, seconds: Decimal) -> impl Future<Output = ()> + Send {
        match self.seconds.lock() {
            Ok(mut guard) => guard.push(seconds),
            Err(poisoned) => poisoned.into_inner().push(seconds),
        }
        ready(())
    }
}

pub(crate) struct ScriptedJitter {
    values: VecDeque<Decimal>,
}

impl ScriptedJitter {
    pub(crate) fn new(values: &[String]) -> Result<Self, Box<HelperError>> {
        let parsed = values
            .iter()
            .map(|text| {
                Decimal::from_str(text).map_err(|error| invalid(format!("{text}: {error}")))
            })
            .collect::<Result<VecDeque<_>, _>>()?;
        Ok(Self { values: parsed })
    }
}

impl JitterSource for ScriptedJitter {
    fn unit_interval(&mut self) -> Decimal {
        self.values.pop_front().unwrap_or(Decimal::ZERO)
    }
}
