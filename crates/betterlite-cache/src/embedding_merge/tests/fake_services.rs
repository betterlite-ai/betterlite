use std::sync::Mutex;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

use crate::cache_key::value::KeyObject;
use crate::embedding_merge::cached::{CacheHitServices, CompleteHit};
use crate::embedding_merge::response::CachedEmbeddingResponse;

#[derive(Debug, Error)]
#[error("the fake service failed")]
pub(super) struct FakeFailure;

pub(super) struct LoggedHit {
    pub model: String,
    pub kwargs: KeyObject,
    pub response: CachedEmbeddingResponse,
    pub start_time: DateTime<Utc>,
}

pub(super) struct FakeServices {
    pub counted: Mutex<Vec<String>>,
    pub logged: Mutex<Vec<LoggedHit>>,
    pub fail_counting: bool,
    pub fail_logging: bool,
}

impl FakeServices {
    pub(super) fn working() -> Self {
        Self {
            counted: Mutex::new(Vec::new()),
            logged: Mutex::new(Vec::new()),
            fail_counting: false,
            fail_logging: false,
        }
    }

    pub(super) fn failing_to_count() -> Self {
        Self {
            fail_counting: true,
            ..Self::working()
        }
    }

    pub(super) fn failing_to_log() -> Self {
        Self {
            fail_logging: true,
            ..Self::working()
        }
    }
}

#[async_trait]
impl CacheHitServices for FakeServices {
    type Error = FakeFailure;

    async fn count_response_tokens(&self, text: &str) -> Result<i64, FakeFailure> {
        if self.fail_counting {
            return Err(FakeFailure);
        }
        self.counted
            .lock()
            .map_err(|_poisoned| FakeFailure)?
            .push(text.to_owned());
        i64::try_from(text.len()).map_err(|_overflow| FakeFailure)
    }

    fn log_complete_hit(&self, hit: &CompleteHit<'_>) -> Result<(), FakeFailure> {
        if self.fail_logging {
            return Err(FakeFailure);
        }
        self.logged
            .lock()
            .map_err(|_poisoned| FakeFailure)?
            .push(LoggedHit {
                model: hit.model.to_owned(),
                kwargs: hit.kwargs.clone(),
                response: hit.response.clone(),
                start_time: hit.start_time,
            });
        Ok(())
    }
}
