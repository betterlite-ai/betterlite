use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;

use super::support::{headers, json, spent_object, upstream};
use crate::passthrough_handler::ports::{BaseConfig, PassthroughLoggingProvider};
use crate::passthrough_handler::types::{
    PassthroughHandlerError, StreamedResponse, UpstreamChatResponse,
};
use crate::passthrough_stream_logging::object::StreamLoggingObject;

struct LeafProvider {
    reply: Option<StreamedResponse>,
}

impl PassthroughLoggingProvider for LeafProvider {
    fn llm_provider_name(&self) -> ProviderId {
        ProviderId::Anthropic
    }

    fn get_provider_config(&self, _model: &str) -> Box<dyn BaseConfig> {
        Box::new(LeafConfig)
    }

    fn build_streaming_response(
        &self,
        _all_chunks: &[String],
        _logging_obj: &StreamLoggingObject,
        _model: &str,
    ) -> Result<Option<StreamedResponse>, PassthroughHandlerError> {
        Ok(self.reply.clone())
    }
}

struct LeafConfig;

impl BaseConfig for LeafConfig {
    fn transform_response(
        &self,
        _upstream: UpstreamChatResponse<'_>,
        _model: &str,
        _logging_obj: &StreamLoggingObject,
    ) -> Result<Json, PassthroughHandlerError> {
        Ok(json(r#"{"id": "leaf-1"}"#))
    }
}

#[test]
fn a_leaf_provides_the_three_members() {
    let provider: &dyn PassthroughLoggingProvider = &LeafProvider { reply: None };
    assert_eq!(provider.llm_provider_name(), ProviderId::Anthropic);
    let logging = spent_object();
    let headers = headers();
    let config = provider.get_provider_config("claude-3");
    let transformed = config
        .transform_response(upstream(&headers), "claude-3", &logging)
        .unwrap();
    assert_eq!(transformed, json(r#"{"id": "leaf-1"}"#));
}

#[test]
fn the_rebuild_may_return_a_model_response() {
    let logging = spent_object();
    let provider = LeafProvider {
        reply: Some(StreamedResponse::Model(json(r#"{"id": "rebuilt-9"}"#))),
    };
    let built = provider
        .build_streaming_response(&["one".to_owned()], &logging, "claude-3")
        .unwrap();
    assert_eq!(
        built,
        Some(StreamedResponse::Model(json(r#"{"id": "rebuilt-9"}"#)))
    );
}

#[test]
fn the_rebuild_may_return_a_text_completion_or_nothing() {
    let logging = spent_object();
    let text = LeafProvider {
        reply: Some(StreamedResponse::TextCompletion(json(
            r#"{"id": "rebuilt-9"}"#,
        ))),
    };
    assert_eq!(
        text.build_streaming_response(&[], &logging, "").unwrap(),
        Some(StreamedResponse::TextCompletion(json(
            r#"{"id": "rebuilt-9"}"#
        )))
    );
    let nothing = LeafProvider { reply: None };
    assert_eq!(
        nothing.build_streaming_response(&[], &logging, "").unwrap(),
        None
    );
}
