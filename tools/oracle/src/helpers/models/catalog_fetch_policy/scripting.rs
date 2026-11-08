use std::collections::VecDeque;
use std::future::{Future, ready};
use std::str::FromStr as _;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_models::fetch_policy::client::{CatalogClient, CatalogRequest, FetchClientError};
use betterlite_models::fetch_policy::response::FetchResponse;
use betterlite_models::fetch_policy::retry::{CatalogSleeper, JitterSource};
use rust_decimal::Decimal;

use super::invalid;
use crate::helpers::HelperError;

pub(super) struct ScriptedJitter {
    values: VecDeque<Decimal>,
}

impl ScriptedJitter {
    pub(super) fn new(values: &[String]) -> Result<Self, Box<HelperError>> {
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

pub(super) struct ScriptedClient {
    pub(super) replies: Vec<Result<FetchResponse, FetchClientError>>,
    pub(super) calls: AtomicUsize,
}

impl ScriptedClient {
    fn next_reply(&self) -> Result<FetchResponse, FetchClientError> {
        let index = self.calls.fetch_add(1, Ordering::SeqCst);
        self.replies.get(index).cloned().unwrap_or_else(|| {
            Err(FetchClientError::Transport {
                detail: "unscripted request".to_owned(),
            })
        })
    }
}

impl CatalogClient for ScriptedClient {
    fn get(&self, _request: &CatalogRequest) -> Result<FetchResponse, FetchClientError> {
        self.next_reply()
    }

    fn get_async(
        &self,
        _request: &CatalogRequest,
    ) -> impl Future<Output = Result<FetchResponse, FetchClientError>> + Send {
        ready(self.next_reply())
    }
}

pub(super) struct RecordingSleeper {
    pub(super) seconds: Mutex<Vec<Decimal>>,
}

impl RecordingSleeper {
    pub(super) fn recorded(&self) -> Vec<Decimal> {
        match self.seconds.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

impl CatalogSleeper for RecordingSleeper {
    fn sleep(&self, seconds: Decimal) -> impl Future<Output = ()> + Send {
        match self.seconds.lock() {
            Ok(mut guard) => guard.push(seconds),
            Err(poisoned) => poisoned.into_inner().push(seconds),
        }
        ready(())
    }
}
