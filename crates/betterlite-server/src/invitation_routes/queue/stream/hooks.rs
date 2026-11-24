use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use betterlite_models::model_registration::JsonObject;

use super::super::request::data_map;
use crate::invitation_routes::state::QueueFailureHook;
use crate::sse_data_generator::types::{
    FailureKind, FailureParam, FailureType, ItemStream, StreamFailure, StreamHooks,
};
use crate::stream_chunks::StreamItem;

mod tests;

pub(super) struct QueueHooks {
    pub(super) failure: Option<Arc<dyn QueueFailureHook>>,
}

#[async_trait]
impl StreamHooks<Principal> for QueueHooks {
    fn needs_iterator_wrap(&self) -> bool {
        false
    }

    fn needs_per_chunk_hook(&self) -> bool {
        false
    }

    fn wrap_iterator(
        &self,
        items: ItemStream,
        _key: &Principal,
        _request_data: &JsonObject,
    ) -> ItemStream {
        items
    }

    async fn chunk_hook(
        &self,
        item: StreamItem,
        _key: &Principal,
        _request_data: &JsonObject,
        _str_so_far: Option<&str>,
    ) -> Result<StreamItem, StreamFailure> {
        Ok(item)
    }

    async fn failure_hook(
        &self,
        key: &Principal,
        failure: &StreamFailure,
        request_data: &JsonObject,
    ) -> Result<(), StreamFailure> {
        let Some(hook) = &self.failure else {
            return Ok(());
        };
        let exception = exception_from_stream(failure);
        match hook
            .post_call_failure(key, &exception, &data_map(request_data))
            .await
        {
            Some(replacement) => Err(StreamFailure {
                message: replacement.message.clone(),
                error_type: FailureType::Text(replacement.error_type.clone()),
                param: replacement
                    .param
                    .as_ref()
                    .map_or(FailureParam::Null, |text| FailureParam::Text(text.clone())),
                status_code: replacement.code.parse::<u16>().ok(),
                code: None,
                body: None,
                kind: FailureKind::Other,
            }),
            None => Ok(()),
        }
    }

    fn fire_deferred_stream_logging(&self, _request_data: &JsonObject) {}
}

fn exception_from_stream(failure: &StreamFailure) -> ProxyException {
    ProxyException::new(ProxyExceptionInit {
        message: failure.message.clone(),
        error_type: match &failure.error_type {
            FailureType::Text(text) => text.clone(),
            FailureType::Missing | FailureType::Null => ProxyErrorTypes::AuthError.to_string(),
        },
        param: match &failure.param {
            FailureParam::Text(text) => Some(text.clone()),
            FailureParam::Missing | FailureParam::Null => None,
        },
        code: failure.status_code.map_or(ExceptionCode::Absent, |status| {
            ExceptionCode::Status(i64::from(status))
        }),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })
}
