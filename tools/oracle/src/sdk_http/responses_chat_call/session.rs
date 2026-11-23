use std::convert::Infallible;
use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_core::api::bfl::poll::PollSleeper;
use betterlite_core::api::responses_session_history::{
    ColdStorageReader, SessionLog, SpendLogQuery,
};
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::super::error::SdkHttpError;

// The case scripts the spend log rows; the handler's own B915 lookup reads them through the query seam
pub(crate) struct RecordedQuery {
    rows: Vec<SessionLog>,
    lookups: Mutex<Vec<String>>,
}

impl RecordedQuery {
    pub(crate) fn new(rows: &[Json]) -> Result<Self, SdkHttpError> {
        let mut decoded = Vec::with_capacity(rows.len());
        for row in rows {
            let value = serde_json::to_value(row).map_err(SdkHttpError::Serialize)?;
            let log = serde_json::from_value(value).map_err(|source| {
                SdkHttpError::Invalid(format!("a spend log row does not read back: {source}"))
            })?;
            decoded.push(log);
        }
        Ok(Self {
            rows: decoded,
            lookups: Mutex::new(Vec::new()),
        })
    }

    // LiteLLM's query takes the response id as its one parameter, so this is the request id it looked up
    pub(crate) fn lookups(&self) -> Vec<String> {
        self.lookups
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl SpendLogQuery for RecordedQuery {
    fn session_logs(
        &self,
        _sql: &'static str,
        request_id: &str,
    ) -> impl Future<Output = Result<Vec<SessionLog>, TableError>> + Send {
        self.lookups
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request_id.to_owned());
        let rows = self.rows.clone();
        ready(Ok(rows))
    }
}

// The case configures no cold storage logger, so the handler's reader is never asked for an object
pub(crate) struct NoCold;

impl ColdStorageReader for NoCold {
    type Error = Infallible;

    fn proxy_server_request(
        &self,
        _object_key: &str,
    ) -> impl Future<Output = Result<Option<IndexMap<String, Json>>, Infallible>> + Send {
        ready(Ok(None))
    }
}

// disable_spend_logs leaves the lookup one attempt, so the retry interval never elapses
pub(crate) struct NoSleep;

impl PollSleeper for NoSleep {
    fn sleep(&self, _duration: Duration) -> impl Future<Output = ()> + Send {
        ready(())
    }
}
