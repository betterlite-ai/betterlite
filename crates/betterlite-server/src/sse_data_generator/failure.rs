use std::io;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_error::envelope::{ErrorObject, ErrorResponse, NoneMarker, WireField};
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use bytes::Bytes;

use super::cleanup::CleanupGuard;
use super::{FailureKind, FailureParam, FailureType, StreamErrorState, StreamFailure, StreamHooks};
use crate::stream_chunks::DONE_FRAME;
use crate::stream_chunks::frames::sse_data_frame;
use betterlite_models::model_registration::JsonObject;

pub(super) struct FailureContext<'a, Key: Sync> {
    pub(super) hooks: &'a dyn StreamHooks<Key>,
    pub(super) request_data: &'a JsonObject,
    pub(super) error_state: &'a mut Option<Box<dyn StreamErrorState + Send>>,
    pub(super) guard: &'a mut CleanupGuard<Key>,
    pub(super) skip_done: bool,
}

impl<Key: Sync> FailureContext<'_, Key> {
    pub(super) async fn run(&mut self, failure: StreamFailure) -> Vec<Result<Bytes, io::Error>> {
        tracing::error!(
            "litellm.proxy.proxy_server.async_data_generator(): Exception occurred - {}",
            failure.message
        );
        let key = &self.guard.outcome.as_ref().expect("active cleanup").key;
        if let Err(hook_error) = self
            .hooks
            .failure_hook(key, &failure, self.request_data)
            .await
        {
            self.guard.complete(false);
            return vec![Err(io::Error::other(hook_error.message))];
        }
        if let Some(state) = self.error_state.as_mut() {
            self.guard.complete(true);
            let mut frames = state
                .format_failure(&failure)
                .into_iter()
                .map(|frame| Ok(Bytes::from(frame)))
                .collect::<Vec<_>>();
            if !self.skip_done {
                frames.push(Ok(Bytes::from(DONE_FRAME)));
            }
            return frames;
        }
        if failure.kind == FailureKind::Http {
            self.guard.complete(false);
            return vec![Err(io::Error::other(failure.message))];
        }
        self.guard.complete(true);
        vec![Ok(Bytes::from(failure_frame(&failure)))]
    }
}

pub(super) fn unix_timestamp() -> i64 {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    i64::try_from(seconds).unwrap_or(i64::MAX)
}

pub(super) fn failure_frame(failure: &StreamFailure) -> String {
    let exception = ProxyException::new(ProxyExceptionInit {
        message: failure.message.clone(),
        error_type: match &failure.error_type {
            FailureType::Missing | FailureType::Null => "None".to_owned(),
            FailureType::Text(value) => value.clone(),
        },
        param: match &failure.param {
            FailureParam::Missing | FailureParam::Null => None,
            FailureParam::Text(value) => Some(value.clone()),
        },
        code: ExceptionCode::Status(i64::from(failure.status_code.unwrap_or(500))),
        ..ProxyExceptionInit::default()
    });
    let error = ErrorObject {
        message: exception.message,
        error_type: match &failure.error_type {
            FailureType::Missing => WireField::Missing(NoneMarker::None),
            FailureType::Null => WireField::Null,
            FailureType::Text(value) => WireField::Text(value.clone()),
        },
        param: match &failure.param {
            FailureParam::Missing => WireField::Missing(NoneMarker::None),
            FailureParam::Null => WireField::Null,
            FailureParam::Text(value) => WireField::Text(value.clone()),
        },
        code: WireField::Text(exception.code),
    };
    let payload = serde_json::to_string(&ErrorResponse { error }).unwrap_or_default();
    sse_data_frame(&payload)
}
