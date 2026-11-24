#![cfg(test)]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_config::sections::settings::JsonValue;
use betterlite_error::proxy_exception::ProxyException;
use betterlite_openai::openai_base_config::Object as JsonMap;

use super::QueueLateFailure;
use crate::first_byte_sse::LateFailureHook as _;
use crate::invitation_routes::state::QueueFailureHook;
use crate::sse_error_frames::SseFailure;

struct RecordingHook {
    calls: Mutex<usize>,
}

impl RecordingHook {
    fn calls(&self) -> usize {
        *self.calls.lock().expect("the hook lock")
    }
}

#[async_trait]
impl QueueFailureHook for RecordingHook {
    async fn post_call_failure(
        &self,
        _caller: &Principal,
        _exception: &ProxyException,
        _request_data: &JsonMap,
    ) -> Option<ProxyException> {
        *self.calls.lock().expect("the hook lock") += 1;
        None
    }
}

fn principal() -> Principal {
    serde_json::from_str(
        r#"{"principal_type":"service_account","subject":"master_key","auth_method":"api_key"}"#,
    )
    .expect("a principal")
}

fn failure() -> SseFailure {
    SseFailure {
        status_code: Some(500),
        code: None,
        error_type: None,
        param: None,
        detail: Some(Box::new(JsonValue::String("boom".to_owned()))),
        provider_specific_fields: None,
    }
}

#[tokio::test]
async fn a_late_failure_runs_the_hook_and_keeps_its_none() {
    let hook = Arc::new(RecordingHook {
        calls: Mutex::new(0),
    });
    let late = QueueLateFailure {
        hook: Arc::clone(&hook),
        caller: principal(),
        data: JsonMap::new(),
    };
    let answered = match late.late_failure(&failure()).await {
        Ok(answered) => answered,
        Err(_raised) => panic!("the test hook answers without raising"),
    };
    assert!(answered.is_none());
    assert_eq!(hook.calls(), 1);
}
