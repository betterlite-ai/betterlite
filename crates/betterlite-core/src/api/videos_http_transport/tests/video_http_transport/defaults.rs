use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::decisions::Json;
use http::HeaderMap;

use super::support::{
    Events, Fixture, Reply, Source, TestError, Wire, body_json, generation_call, headers, params,
    video_deps,
};
use crate::api::video_records::VideoObject;
use crate::api::videos_http_transport::video_config::{
    Params, RequestFiles, VideoEnvironment, VideoHttpConfig, VideoLogging, VideoReply,
    VideoRequestContext,
};

// A config that fills only the abstract transforms, so every base default is what answers
struct Plain {
    base: String,
}

impl Plain {
    fn new() -> Self {
        Self {
            base: "http://plain.test/v1".to_owned(),
        }
    }
}

impl VideoHttpConfig for Plain {
    type Error = TestError;

    fn get_complete_url(
        &self,
        _model: &str,
        _api_base: Option<&str>,
        _litellm_params: &Params,
    ) -> Result<String, TestError> {
        Ok(self.base.clone())
    }

    fn transform_video_create_request(
        &self,
        _model: &str,
        _prompt: &str,
        _optional_params: &Params,
        context: VideoRequestContext<'_>,
    ) -> Result<(Params, RequestFiles, String), TestError> {
        Ok((
            params(r#"{"prompt":"a cat"}"#),
            Vec::new(),
            format!("{}/submit", context.api_base),
        ))
    }

    fn transform_video_create_response(
        &self,
        _model: &str,
        _reply: VideoReply<'_>,
        _request_data: Option<&Params>,
    ) -> Result<VideoObject, TestError> {
        Ok(VideoObject::new("video-1".to_owned(), "queued".to_owned()))
    }

    fn transform_video_content_request(
        &self,
        video_id: &str,
        _variant: Option<&str>,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), TestError> {
        Ok((
            format!("{}/contents/{video_id}", context.api_base),
            Params::new(),
        ))
    }

    fn transform_video_content_response(
        &self,
        raw_response: &UpstreamResponse,
        logging_obj: &mut VideoLogging,
    ) -> Result<Vec<u8>, TestError> {
        logging_obj
            .model_call_details
            .insert("plain_sync_mark".to_owned(), Json::Bool(true));
        Ok(raw_response.body.clone())
    }
}

#[test]
fn the_base_environment_starts_from_nothing() {
    let given = headers(&[("X-Given", "given")]);
    let environment = VideoEnvironment {
        headers: &given,
        model: "m",
        api_key: None,
        litellm_params: None,
    };
    assert!(Plain::new().validate_environment(environment).is_empty());
}

#[test]
fn the_base_asks_for_json_not_form_data() {
    assert!(!Plain::new().use_multipart_form_data());
}

#[test]
fn the_base_error_facts_name_the_error_and_claim_no_status() {
    let facts = Plain::new().error_facts(&TestError::Transform);
    assert_eq!(facts.message, "the transform refused");
    assert_eq!(facts.status_code, None);
    assert!(facts.headers.is_empty());
}

#[test]
fn the_base_error_class_keeps_the_message_the_status_and_the_headers() {
    let error = Plain::new().get_error_class("boom", 418, &headers(&[("x-reason", "teapot")]));
    assert_eq!(error.status_code, 418);
    assert_eq!(error.message, "boom");
    assert_eq!(
        error.headers,
        Some(vec![("x-reason".to_owned(), "teapot".to_owned())])
    );
    assert!(!error.status_code_is_synthesized);
}

#[tokio::test]
async fn the_base_async_content_transform_answers_with_the_sync_one() {
    let response = UpstreamResponse {
        status: 200,
        headers: HeaderMap::new(),
        body: b"bytes".to_vec(),
    };
    let mut logging = VideoLogging::default();
    let bytes = Plain::new()
        .async_transform_video_content_response(&response, &mut logging)
        .await
        .expect("the bytes");
    assert_eq!(bytes.as_slice(), b"bytes");
    assert_eq!(
        logging.model_call_details.get("plain_sync_mark"),
        Some(&Json::Bool(true))
    );
}

#[tokio::test]
async fn a_plain_config_sends_json_with_the_headers_the_base_gives_it() {
    let events = Events::new();
    let wire = Wire::replying(Reply::new(200, "{}"), &events);
    let source = Source::new(wire.clone());
    let config = Plain::new();
    let deps = video_deps(&config, &source, None);
    let mut fixture = Fixture::new();
    let optional = Params::new();
    let litellm = Params::new();
    let call = generation_call(&optional, &litellm);
    fixture
        .generate(&deps, &call, &events, false)
        .await
        .expect("the video");
    let requests = wire.requests();
    assert!(requests[0].headers.is_empty());
    assert_eq!(
        body_json(&requests[0]),
        Json::Object(params(r#"{"prompt":"a cat"}"#))
    );
}
