use rstest::rstest;

use super::fixtures::{Environment, ScriptedFailure, litellm_params, request};
use super::fixtures::{Scripted, forwarded_fields};
use crate::api::genai::chat_call::{
    ChatCallError, ForwardedFields, GenerateContentCall, generate_content_handler,
    prepare_completion_call,
};
use crate::api::genai_chat_input::GenAiInputError;
use crate::api::genai_chat_input::input::GenerateContentRequest;
use crate::schema::SchemaLimits;

const REQUEST_WITH_TOOL: &str = r#"{"model":"gpt-4o","contents":[],"tools":[{"functionDeclarations":[{"name":"lookup","parameters":{"type":"object","properties":{"a":{"type":"string"},"b":{"type":"string"}}}}]}]}"#;

#[test]
fn prepare_forwards_metadata_extra_headers_and_proxy_server_request() {
    let environment = Environment::new();
    let resolution = environment.resolution();
    let context = environment.context(&resolution);
    let request = request();
    let params = litellm_params();
    let prepared = prepare_completion_call(GenerateContentCall {
        request: &request,
        litellm_params: &params,
        context: &context,
        stream: false,
        forwarded: forwarded_fields(),
    })
    .expect("the prepared call");
    assert_eq!(prepared.forwarded, forwarded_fields());
    assert!(prepared.forwarded.metadata.is_some());
    assert!(prepared.forwarded.extra_headers.is_some());
    assert!(prepared.forwarded.proxy_server_request.is_some());
}

#[test]
fn prepare_leaves_the_forwarded_fields_absent_when_the_caller_sent_none() {
    let environment = Environment::new();
    let resolution = environment.resolution();
    let context = environment.context(&resolution);
    let request = request();
    let params = litellm_params();
    let prepared = prepare_completion_call(GenerateContentCall {
        request: &request,
        litellm_params: &params,
        context: &context,
        stream: false,
        forwarded: ForwardedFields::default(),
    })
    .expect("the prepared call");
    assert_eq!(prepared.forwarded, ForwardedFields::default());
}

#[test]
fn prepare_translates_the_request_and_keeps_the_litellm_params() {
    let environment = Environment::new();
    let resolution = environment.resolution();
    let context = environment.context(&resolution);
    let request = request();
    let params = litellm_params();
    let prepared = prepare_completion_call(GenerateContentCall {
        request: &request,
        litellm_params: &params,
        context: &context,
        stream: false,
        forwarded: ForwardedFields::default(),
    })
    .expect("the prepared call");
    assert_eq!(prepared.request.model.as_deref(), Some("gpt-4o"));
    assert_eq!(prepared.request.messages.len(), 1);
    let kept = prepared.litellm_params.expect("the kept LiteLLM params");
    assert_eq!(kept.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(kept.max_retries, Some(2));
}

#[rstest]
#[case::streaming(true)]
#[case::not_streaming(false)]
fn prepare_sets_stream_to_what_the_caller_asked_for(#[case] stream: bool) {
    let environment = Environment::new();
    let resolution = environment.resolution();
    let context = environment.context(&resolution);
    let request = request();
    let params = litellm_params();
    let prepared = prepare_completion_call(GenerateContentCall {
        request: &request,
        litellm_params: &params,
        context: &context,
        stream,
        forwarded: ForwardedFields::default(),
    })
    .expect("the prepared call");
    assert_eq!(prepared.request.stream, stream);
}

#[test]
fn an_input_failure_is_not_wrapped_and_skips_the_completion() {
    let environment = Environment::limited(SchemaLimits {
        max_schema_bytes: 1_048_576,
        max_response_bytes: 1_048_576,
        max_depth: 100,
        max_nodes: 3,
    });
    let resolution = environment.resolution();
    let context = environment.context(&resolution);
    let request =
        serde_json::from_str::<GenerateContentRequest>(REQUEST_WITH_TOOL).expect("a GenAI request");
    let params = litellm_params();
    let executor = Scripted::answering(Err(ScriptedFailure("unused")));
    let handled = generate_content_handler(
        &executor,
        GenerateContentCall {
            request: &request,
            litellm_params: &params,
            context: &context,
            stream: false,
            forwarded: ForwardedFields::default(),
        },
        false,
    );
    let Err(error) = handled else {
        panic!("the translation should have failed");
    };
    assert!(matches!(
        error,
        ChatCallError::Input(GenAiInputError::Schema(_))
    ));
    assert!(executor.seen().is_empty());
}
