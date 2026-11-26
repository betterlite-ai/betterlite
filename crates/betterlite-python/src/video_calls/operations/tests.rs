#![cfg(test)]

use std::time::Duration;

use betterlite_core::api::video_generation_call::VideoGenerationRequest;
use betterlite_core::api::video_generation_call::call::VideoGenerationCaller;
use betterlite_core::api::video_generation_call::handler::TransportGenerationHandler;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::{mock_object, providers, redactor, request_timeout};
use crate::video_calls::convert::VideoCallWire;
use crate::video_calls::error::RunError;
use crate::video_calls::session::VideoSession;

#[test]
fn configured_request_timeout() {
    let session = VideoSession::build(r#"{"environment":{"REQUEST_TIMEOUT":"42"}}"#).unwrap();
    assert_eq!(request_timeout(&session).unwrap(), Duration::from_secs(42));
}

#[test]
fn malformed_mock_keeps_model() {
    let session = VideoSession::build(r#"{"environment":{}}"#).unwrap();
    let caller = VideoGenerationCaller {
        providers: providers(&session, &VideoCallWire::default(), None),
        handler: TransportGenerationHandler,
        redactor: redactor().unwrap(),
        request_timeout: Duration::from_secs(60),
        drop_params: false,
    };
    let mut request = VideoGenerationRequest::new("p".to_owned());
    request.model = Some("caller-model".to_owned());
    request.custom_llm_provider = Some("openai".to_owned());
    request.kwargs = IndexMap::from([(
        "mock_response".to_owned(),
        Json::Text("not json".to_owned()),
    )]);
    let RunError::Call(error) = mock_object(&caller, &request).unwrap_err() else {
        panic!("the malformed mock must become a call failure");
    };
    assert_eq!(error.model.as_deref(), Some("caller-model"));
}
