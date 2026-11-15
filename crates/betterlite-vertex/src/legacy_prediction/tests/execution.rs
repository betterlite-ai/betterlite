use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use betterlite_models::decisions::{Extra, Json};
use futures_util::StreamExt as _;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;

use crate::legacy_prediction::client::options::{PredictCall, parameters};
use crate::legacy_prediction::client::{
    EndpointPredictRequest, chat_predict_body, chat_streaming_body, endpoint_predict_body,
    text_predict_body, text_streaming_body,
};
use crate::legacy_prediction::error::{LegacyPredictionError, PredictionError};
use crate::legacy_prediction::execution::{LegacyContext, complete, stream as execute_stream};
use crate::legacy_prediction::invocation::LegacyInvocation;
use crate::legacy_prediction::projection::LegacyMessage;

use super::support::{
    Call, CancellingStream, FakeEncoding, FixtureClient, access, access_lookup, chat_invocation,
    counts, custom_invocation, empty_stream, float_number, json, legacy_context, number, options,
    predictions_reply, projection_services, text_invocation, text_reply, vision_invocation,
    words_of,
};

async fn streamed_words(
    client: &FixtureClient,
    invocation: LegacyInvocation,
    context: &LegacyContext<'_, '_>,
) -> Vec<String> {
    let chunks = execute_stream(client, invocation, context, access_lookup())
        .await
        .expect("a stream");
    words_of(chunks).await
}

fn one_endpoint_call(client: &FixtureClient) -> EndpointPredictRequest {
    let calls = client.calls();
    let [Call::Endpoint(request)] = calls.as_slice() else {
        panic!("one endpoint callback");
    };
    request.clone()
}

#[tokio::test]
async fn execution_resolves_tokens() {
    let encoding = FakeEncoding {
        counts: IndexMap::new(),
    };
    let services = projection_services(&encoding);
    let context = legacy_context(&services);
    let lookups = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&lookups);
    let lookup = async move {
        counter.fetch_add(1, Ordering::SeqCst);
        Ok(access())
    };
    let client = FixtureClient::with_text(text_reply("ok"));
    complete(&client, chat_invocation(Extra::new()), &context, lookup)
        .await
        .expect("the chat completion");
    assert_eq!(lookups.load(Ordering::SeqCst), 1);
    let calls = client.calls();
    let [Call::Message(request)] = calls.as_slice() else {
        panic!("one chat call");
    };
    assert_eq!(request.token.expose_secret(), "resolved-token");
    assert_eq!(request.project, "proj");
}

#[tokio::test]
async fn execution_falls_back_to_the_token_project() {
    let encoding = FakeEncoding {
        counts: IndexMap::new(),
    };
    let services = projection_services(&encoding);
    let context = LegacyContext {
        project: None,
        location: Some("us-central1"),
        model_is_language: false,
        services: &services,
    };
    let client = FixtureClient::with_text(text_reply("ok"));
    complete(
        &client,
        chat_invocation(Extra::new()),
        &context,
        access_lookup(),
    )
    .await
    .expect("the chat completion");
    let calls = client.calls();
    let [Call::Message(request)] = calls.as_slice() else {
        panic!("one chat call");
    };
    assert_eq!(request.project, "resolved-project");
}

#[tokio::test]
async fn execution_completion_failure_is_vertex_500() {
    let encoding = FakeEncoding {
        counts: IndexMap::new(),
    };
    let services = projection_services(&encoding);
    let context = legacy_context(&services);
    let client = FixtureClient::with_text_failure(PredictionError::Http {
        status: 404,
        message: String::from("missing"),
    });
    let error = complete(
        &client,
        chat_invocation(Extra::new()),
        &context,
        access_lookup(),
    )
    .await
    .expect_err("a failed completion");
    assert_eq!(error.status_code(), 500);
    assert!(matches!(
        error,
        LegacyPredictionError::Completion {
            source: PredictionError::Http { status: 404, .. }
        }
    ));
}

#[tokio::test]
async fn execution_endpoint_failure_is_vertex_500() {
    let encoding = FakeEncoding {
        counts: IndexMap::new(),
    };
    let services = projection_services(&encoding);
    let context = legacy_context(&services);
    let client = FixtureClient::with_endpoint_failure(PredictionError::Http {
        status: 404,
        message: String::from("missing"),
    });
    let error = complete(
        &client,
        custom_invocation(vec![json("{\"prompt\":\"hello\"}")]),
        &context,
        access_lookup(),
    )
    .await
    .expect_err("a failed endpoint completion");
    assert_eq!(error.status_code(), 500);
    assert!(matches!(
        error,
        LegacyPredictionError::Completion {
            source: PredictionError::Http { status: 404, .. }
        }
    ));
}

#[tokio::test]
async fn execution_streaming_failure_is_vertex_500() {
    let encoding = FakeEncoding {
        counts: IndexMap::new(),
    };
    let services = projection_services(&encoding);
    let context = legacy_context(&services);
    let client = FixtureClient::with_stream_failure(PredictionError::Http {
        status: 404,
        message: String::from("missing"),
    });
    let Err(error) = execute_stream(
        &client,
        chat_invocation(Extra::new()),
        &context,
        access_lookup(),
    )
    .await
    else {
        panic!("a failed stream");
    };
    assert_eq!(error.status_code(), 500);
    assert!(matches!(
        error,
        LegacyPredictionError::Streaming {
            source: PredictionError::Http { status: 404, .. }
        }
    ));
}

#[tokio::test]
async fn execution_unhandled_mode_cannot_generate() {
    let encoding = FakeEncoding {
        counts: IndexMap::new(),
    };
    let services = projection_services(&encoding);
    let context = legacy_context(&services);
    let client = FixtureClient::default();
    let Err(error) = execute_stream(&client, vision_invocation(), &context, access_lookup()).await
    else {
        panic!("the vision route has no stream");
    };
    assert!(matches!(error, LegacyPredictionError::UnableToGenerate));
    assert_eq!(error.to_string(), "Unable to generate response");
    assert_eq!(error.status_code(), 500);
    assert!(client.calls().is_empty());
}

#[tokio::test]
async fn execution_custom_requires_project_and_location() {
    let encoding = FakeEncoding {
        counts: IndexMap::new(),
    };
    let services = projection_services(&encoding);
    let context = LegacyContext {
        project: Some("proj"),
        location: None,
        model_is_language: false,
        services: &services,
    };
    let client = FixtureClient::with_endpoint(predictions_reply(&["first"]));
    let error = complete(
        &client,
        custom_invocation(vec![json("{\"prompt\":\"hello\"}")]),
        &context,
        access_lookup(),
    )
    .await
    .expect_err("a custom call needs a location");
    assert!(matches!(
        error,
        LegacyPredictionError::Completion {
            source: PredictionError::CustomEndpointContext
        }
    ));
    assert_eq!(error.status_code(), 500);
}

#[tokio::test]
async fn execution_private_requires_instances() {
    let encoding = FakeEncoding {
        counts: IndexMap::new(),
    };
    let services = projection_services(&encoding);
    let context = legacy_context(&services);
    let client = FixtureClient::with_stream(empty_stream());
    let Err(error) = execute_stream(
        &client,
        private_invocation(Vec::new()),
        &context,
        access_lookup(),
    )
    .await
    else {
        panic!("no private instance");
    };
    assert!(matches!(
        error,
        LegacyPredictionError::Streaming {
            source: PredictionError::MissingInstances
        }
    ));
    assert_eq!(error.status_code(), 500);
}

#[tokio::test]
async fn execution_cancellation() {
    let encoding = FakeEncoding {
        counts: IndexMap::new(),
    };
    let services = projection_services(&encoding);
    let context = legacy_context(&services);
    let cancelled = Arc::new(AtomicBool::new(false));
    let stream = CancellingStream {
        cancelled: Arc::clone(&cancelled),
        chunks: VecDeque::new(),
    }
    .boxed();
    let client = FixtureClient::with_stream(stream);
    let chunks = execute_stream(
        &client,
        chat_invocation(Extra::new()),
        &context,
        access_lookup(),
    )
    .await
    .expect("a live stream");
    assert!(!cancelled.load(Ordering::SeqCst));
    drop(chunks);
    assert!(cancelled.load(Ordering::SeqCst));
}

#[tokio::test]
async fn execution_option_mutation() {
    let encoding = FakeEncoding {
        counts: IndexMap::new(),
    };
    let services = projection_services(&encoding);
    let context = legacy_context(&services);
    let streamed = options(&[
        ("stream", Json::Bool(true)),
        ("temperature", float_number(0.5)),
    ]);

    let chat = FixtureClient::with_stream(empty_stream());
    let chunks = execute_stream(
        &chat,
        chat_invocation(streamed.clone()),
        &context,
        access_lookup(),
    )
    .await
    .expect("a chat stream");
    drop(chunks);
    let calls = chat.calls();
    let [Call::MessageStream(request)] = calls.as_slice() else {
        panic!("one chat stream call");
    };
    assert!(!request.options.contains_key("stream"));
    assert_eq!(request.options.get("temperature"), Some(&float_number(0.5)));

    let text = FixtureClient::with_stream(empty_stream());
    let text_chunks = execute_stream(&text, text_invocation(streamed), &context, access_lookup())
        .await
        .expect("a text stream");
    drop(text_chunks);
    let text_calls = text.calls();
    let [Call::TextStream(text_request)] = text_calls.as_slice() else {
        panic!("one text stream call");
    };
    assert!(!text_request.options.contains_key("stream"));
}

#[tokio::test]
async fn execution_private_instance_mutation() {
    let encoding = FakeEncoding {
        counts: IndexMap::new(),
    };
    let services = projection_services(&encoding);
    let context = legacy_context(&services);
    let instances = vec![json("{\"prompt\":\"hello\",\"stream\":true}")];

    let private = FixtureClient::with_endpoint(predictions_reply(&["first second"]));
    assert_eq!(
        streamed_words(&private, private_invocation(instances.clone()), &context).await,
        vec![String::from("first"), String::from("second")]
    );
    assert_eq!(
        one_endpoint_call(&private).instances,
        vec![json("{\"prompt\":\"hello\"}")]
    );

    let custom = FixtureClient::with_endpoint(predictions_reply(&["first second"]));
    assert_eq!(
        streamed_words(&custom, custom_invocation(instances.clone()), &context).await,
        vec![String::from("first"), String::from("second")]
    );
    assert_eq!(one_endpoint_call(&custom).instances, instances);
}

#[tokio::test]
async fn execution_publisher_completion_selection() {
    let encoding = FakeEncoding {
        counts: counts(&[("hello", 1), ("None", 1)]),
    };
    let services = projection_services(&encoding);
    let context = legacy_context(&services);

    let chat = FixtureClient::with_text(text_reply("ok"));
    complete(
        &chat,
        chat_invocation(Extra::new()),
        &context,
        access_lookup(),
    )
    .await
    .expect("the chat completion");
    assert!(matches!(chat.calls().as_slice(), [Call::Message(_)]));

    let text = FixtureClient::with_text(text_reply("ok"));
    complete(
        &text,
        text_invocation(Extra::new()),
        &context,
        access_lookup(),
    )
    .await
    .expect("the text completion");
    let text_calls = text.calls();
    let [Call::Text(text_request)] = text_calls.as_slice() else {
        panic!("one text call");
    };
    assert_eq!(text_request.model, "text-bison");
}

#[tokio::test]
async fn execution_endpoint_completion_selection() {
    let encoding = FakeEncoding {
        counts: counts(&[("hello", 1), ("None", 1)]),
    };
    let services = projection_services(&encoding);
    let context = legacy_context(&services);

    let custom = FixtureClient::with_endpoint(predictions_reply(&["first"]));
    complete(
        &custom,
        custom_invocation(vec![json("{\"prompt\":\"hello\"}")]),
        &context,
        access_lookup(),
    )
    .await
    .expect("the custom completion");
    assert_eq!(
        one_endpoint_call(&custom).endpoint,
        "projects/proj/locations/us-central1/endpoints/model-garden"
    );

    let private = FixtureClient::with_endpoint(predictions_reply(&["first"]));
    complete(
        &private,
        private_invocation(vec![json("{\"prompt\":\"hello\"}")]),
        &context,
        access_lookup(),
    )
    .await
    .expect("the private completion");
    assert_eq!(
        one_endpoint_call(&private).endpoint,
        "projects/proj/locations/us-central1/endpoints/5678"
    );

    let vision = FixtureClient::default();
    complete(&vision, vision_invocation(), &context, access_lookup())
        .await
        .expect("the vision completion");
    assert!(vision.calls().is_empty());
}

#[tokio::test]
async fn execution_publisher_stream_selection() {
    let encoding = FakeEncoding {
        counts: counts(&[("hello", 1), ("None", 1)]),
    };
    let services = projection_services(&encoding);
    let context = legacy_context(&services);

    let chat = FixtureClient::with_stream(empty_stream());
    drop(
        execute_stream(
            &chat,
            chat_invocation(Extra::new()),
            &context,
            access_lookup(),
        )
        .await
        .expect("the chat stream"),
    );
    assert!(matches!(chat.calls().as_slice(), [Call::MessageStream(_)]));

    let text = FixtureClient::with_stream(empty_stream());
    drop(
        execute_stream(
            &text,
            text_invocation(Extra::new()),
            &context,
            access_lookup(),
        )
        .await
        .expect("the text stream"),
    );
    assert!(matches!(text.calls().as_slice(), [Call::TextStream(_)]));
}

#[tokio::test]
async fn execution_endpoint_stream_selection() {
    let encoding = FakeEncoding {
        counts: counts(&[("hello", 1), ("None", 1)]),
    };
    let services = projection_services(&encoding);
    let context = legacy_context(&services);

    let custom = FixtureClient::with_endpoint(predictions_reply(&["first second"]));
    assert_eq!(
        streamed_words(
            &custom,
            custom_invocation(vec![json("{\"prompt\":\"hello\"}")]),
            &context
        )
        .await,
        vec![String::from("first"), String::from("second")]
    );
    assert_eq!(
        one_endpoint_call(&custom).endpoint,
        "projects/proj/locations/us-central1/endpoints/model-garden"
    );

    let private = FixtureClient::with_endpoint(predictions_reply(&["first second"]));
    assert_eq!(
        streamed_words(
            &private,
            private_invocation(vec![json("{\"prompt\":\"hello\"}")]),
            &context
        )
        .await,
        vec![String::from("first"), String::from("second")]
    );
    assert_eq!(
        one_endpoint_call(&private).endpoint,
        "projects/proj/locations/us-central1/endpoints/5678"
    );
}

#[tokio::test]
async fn execution_endpoint_completion_strips_the_output_separator() {
    let encoding = FakeEncoding {
        counts: IndexMap::new(),
    };
    let services = projection_services(&encoding);
    let context = legacy_context(&services);
    let prediction = "prompt\nOutput:\nanswer";
    let custom = FixtureClient::with_endpoint(predictions_reply(&[prediction]));
    let custom_result = complete(
        &custom,
        custom_invocation(vec![json("{\"prompt\":\"hello\"}")]),
        &context,
        access_lookup(),
    )
    .await
    .expect("the custom completion");
    assert_eq!(
        custom_result.message,
        LegacyMessage::Content(String::from("answer"))
    );
    let private = FixtureClient::with_endpoint(predictions_reply(&[prediction]));
    let private_result = complete(
        &private,
        private_invocation(vec![json("{\"prompt\":\"hello\"}")]),
        &context,
        access_lookup(),
    )
    .await
    .expect("the private completion");
    assert_eq!(
        private_result.message,
        LegacyMessage::Content(String::from("answer"))
    );
}

#[tokio::test]
async fn execution_endpoint_stream_strips_the_output_separator() {
    let encoding = FakeEncoding {
        counts: IndexMap::new(),
    };
    let services = projection_services(&encoding);
    let context = legacy_context(&services);
    let custom = FixtureClient::with_endpoint(predictions_reply(&["prompt\nOutput:\nanswer"]));
    let custom_stream = execute_stream(
        &custom,
        custom_invocation(vec![json("{\"prompt\":\"hello\"}")]),
        &context,
        access_lookup(),
    )
    .await
    .expect("the custom stream");
    assert_eq!(words_of(custom_stream).await, vec![String::from("answer")]);
    let private = FixtureClient::with_endpoint(predictions_reply(&["prompt\nOutput:\nanswer"]));
    let private_stream = execute_stream(
        &private,
        private_invocation(vec![json("{\"prompt\":\"hello\"}")]),
        &context,
        access_lookup(),
    )
    .await
    .expect("the private stream");
    assert_eq!(words_of(private_stream).await, vec![String::from("answer")]);
}

#[tokio::test]
async fn execution_empty_predictions_is_missing() {
    let encoding = FakeEncoding {
        counts: IndexMap::new(),
    };
    let services = projection_services(&encoding);
    let context = legacy_context(&services);
    let client = FixtureClient::with_endpoint(predictions_reply(&[]));
    let error = complete(
        &client,
        custom_invocation(vec![json("{\"prompt\":\"hello\"}")]),
        &context,
        access_lookup(),
    )
    .await
    .expect_err("an empty reply");
    assert!(matches!(
        error,
        LegacyPredictionError::Completion {
            source: PredictionError::MissingPrediction
        }
    ));
    assert_eq!(error.status_code(), 500);
}

#[tokio::test]
async fn execution_empty_stream_predictions_is_missing() {
    let encoding = FakeEncoding {
        counts: IndexMap::new(),
    };
    let services = projection_services(&encoding);
    let context = legacy_context(&services);
    let client = FixtureClient::with_endpoint(predictions_reply(&[]));
    let Err(error) = execute_stream(
        &client,
        custom_invocation(vec![json("{\"prompt\":\"hello\"}")]),
        &context,
        access_lookup(),
    )
    .await
    else {
        panic!("no prediction to stream");
    };
    assert!(matches!(
        error,
        LegacyPredictionError::Streaming {
            source: PredictionError::MissingPrediction
        }
    ));
    assert_eq!(error.status_code(), 500);
}

fn case<'fixture>(cases: &'fixture IndexMap<String, Json>, name: &str) -> &'fixture Json {
    cases.get(name).expect("a fixture case")
}

fn body_json(bytes: &[u8]) -> Json {
    serde_json::from_slice(bytes).expect("a JSON body")
}

fn private_invocation(instances: Vec<Json>) -> LegacyInvocation {
    LegacyInvocation::PrivateEndpoint {
        model: "deployed-model".to_owned(),
        prompt: "hello".to_owned(),
        endpoint: "projects/proj/locations/us-central1/endpoints/5678".to_owned(),
        instances,
        options: Extra::new(),
    }
}

#[test]
fn execution_chat_predict_bodies_match_sdk_fixture() {
    let Json::Object(cases) = json(include_str!("../fixtures/sdk-requests.json")) else {
        panic!("a fixture object");
    };
    assert_eq!(
        &body_json(&chat_predict_body("Hello", &Extra::new()).expect("chat")),
        case(&cases, "chat_predict_defaults")
    );
    assert_eq!(
        &body_json(
            &chat_predict_body(
                "Hello",
                &options(&[
                    ("max_output_tokens", number(8)),
                    ("temperature", float_number(0.5)),
                    ("top_k", number(5)),
                    ("top_p", float_number(0.9)),
                    ("stop_sequences", json("[\"z\"]")),
                    ("candidate_count", number(1)),
                ]),
            )
            .expect("chat")
        ),
        case(&cases, "chat_predict_options")
    );
}

#[test]
fn execution_text_predict_bodies_match_sdk_fixture() {
    let Json::Object(cases) = json(include_str!("../fixtures/sdk-requests.json")) else {
        panic!("a fixture object");
    };
    assert_eq!(
        &body_json(&text_predict_body("Hello", &Extra::new()).expect("text")),
        case(&cases, "text_predict_defaults")
    );
    assert_eq!(
        &body_json(
            &text_predict_body(
                "Hello",
                &options(&[
                    ("max_output_tokens", number(16)),
                    ("temperature", float_number(0.5)),
                    ("top_k", number(40)),
                    ("top_p", float_number(0.9)),
                    ("stop_sequences", json("[\"x\"]")),
                    ("candidate_count", number(2)),
                    ("logprobs", number(1)),
                    ("presence_penalty", float_number(0.1)),
                    ("frequency_penalty", float_number(0.2)),
                    ("logit_bias", json("{\"1\":0.5}")),
                    ("seed", number(7)),
                ]),
            )
            .expect("text")
        ),
        case(&cases, "text_predict_options")
    );
}

#[test]
fn execution_endpoint_and_streaming_bodies_match_sdk_fixture() {
    let Json::Object(cases) = json(include_str!("../fixtures/sdk-requests.json")) else {
        panic!("a fixture object");
    };
    assert_eq!(
        &body_json(
            &endpoint_predict_body(&[json("{\"prompt\":\"Hello\",\"temperature\":0}")])
                .expect("endpoint")
        ),
        case(&cases, "endpoint_predict")
    );
    assert_eq!(
        &body_json(&text_streaming_body("Hi", &Extra::new()).expect("text stream")),
        case(&cases, "text_streaming_defaults")
    );
    assert_eq!(
        &body_json(&chat_streaming_body("Hi", &Extra::new()).expect("chat stream")),
        case(&cases, "chat_streaming_defaults")
    );
    assert_eq!(
        &body_json(
            &chat_streaming_body("Hi", &options(&[("temperature", float_number(0.5))]))
                .expect("chat stream")
        ),
        case(&cases, "chat_streaming_options")
    );
}

#[test]
fn execution_rejects_unknown_options() {
    assert!(matches!(
        parameters(
            &options(&[("presence_penalty", float_number(0.1))]),
            PredictCall::Chat
        ),
        Err(PredictionError::UnsupportedOption { .. })
    ));
    assert!(matches!(
        parameters(
            &options(&[("candidate_count", number(1))]),
            PredictCall::TextStreaming
        ),
        Err(PredictionError::UnsupportedOption { .. })
    ));
}

#[test]
fn execution_rejects_wrong_option_types() {
    assert!(matches!(
        parameters(
            &options(&[("temperature", json("\"hot\""))]),
            PredictCall::Text
        ),
        Err(PredictionError::OptionType { .. })
    ));
    assert!(matches!(
        parameters(&options(&[("top_k", float_number(1.5))]), PredictCall::Text),
        Err(PredictionError::OptionType { .. })
    ));
}
