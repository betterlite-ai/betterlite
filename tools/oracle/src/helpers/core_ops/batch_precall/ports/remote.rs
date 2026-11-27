use std::future::{Future, ready};
use std::sync::PoisonError;

use betterlite_core::api::batch_precall::ports::{BatchCacheError, BatchPrecallRemote};
use betterlite_core::api::request_destination::RequestData;
use betterlite_models::decisions::Json;

use super::super::{Function, Scripted};
use super::cache_error;

impl BatchPrecallRemote for Scripted {
    fn is_redis(&self) -> bool {
        true
    }

    fn cache_key(&self, _data: &RequestData) -> Option<String> {
        None
    }

    fn scan(
        &self,
        _pattern: &str,
        _count: usize,
    ) -> impl Future<Output = Result<Vec<String>, BatchCacheError>> + Send {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.effects.push("scan");
        if let Some(error) = cache_error(&self.remote.faults, Function::Scan) {
            return ready(Err(error));
        }
        ready(Ok(self.remote.scan_keys.clone()))
    }

    fn batch_get(
        &self,
        _keys: &[String],
    ) -> impl Future<Output = Result<Vec<(String, Json)>, BatchCacheError>> + Send {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.effects.push("batch_get");
        let entries = self
            .remote
            .batch_entries
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        if let Some(error) = cache_error(&self.remote.faults, Function::BatchGet) {
            return ready(Err(error));
        }
        ready(Ok(entries))
    }

    fn get(
        &self,
        _key: &str,
    ) -> impl Future<Output = Result<Option<Json>, BatchCacheError>> + Send {
        ready(remote_get_answer(self))
    }
}

// the backing store's read: record the call, then the scripted fault or the stored value
fn remote_get_answer(scripted: &Scripted) -> Result<Option<Json>, BatchCacheError> {
    let mut state = scripted
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    state.effects.push("get");
    match cache_error(&scripted.remote.faults, Function::Get) {
        Some(error) => Err(error),
        None => Ok(scripted.remote.get_value.clone()),
    }
}
