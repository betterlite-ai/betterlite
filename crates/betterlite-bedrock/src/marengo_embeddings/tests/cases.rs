use betterlite_context::non_default_params::mapper::DropPolicy;
use rstest::rstest;
use serde::Deserialize;
use serde_json::Value;

use crate::image_models::params::Params;
use crate::marengo_embeddings::error::MarengoEmbeddingsError;
use crate::marengo_embeddings::params::{
    MARENGO_2_7_ONLY_PARAMS, MARENGO_SHARED_PARAMS, drop_params_enabled, map_openai_params,
    supported_openai_params,
};
use crate::marengo_embeddings::request::{TransformRequest, transform_request};
use crate::marengo_embeddings::response::{
    MarengoBilledRequest, MarengoInvokeResponse, billed_usage, transform_async_invoke_response,
    transform_response,
};
use crate::marengo_three::error::Marengo3Error;
use crate::marengo_three::is_marengo_3_model;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct Scenario {
    model: Option<String>,
    input: String,
    inference: Params,
    non_default: Params,
    optional: Params,
    async_invoke_route: bool,
    model_id: Option<String>,
    output_s3_uri: Option<String>,
    drop_params: bool,
    replies: Vec<Value>,
    batch: Vec<Value>,
    global_drop_params: bool,
    deployment_drop_params: Option<bool>,
}

enum Check {
    Supported { marengo_3: bool },
    Mapped(&'static str),
    Request(&'static str),
    RequestError(&'static str),
    Response(&'static str),
    ReplyError,
    BatchError,
    Usage(&'static str),
    Enabled(bool),
}

fn transform(scenario: &Scenario) -> Result<Value, MarengoEmbeddingsError> {
    let request = transform_request(TransformRequest {
        model: scenario.model.as_deref(),
        input: &scenario.input,
        inference_params: &scenario.inference,
        async_invoke_route: scenario.async_invoke_route,
        model_id: scenario.model_id.as_deref(),
        output_s3_uri: scenario.output_s3_uri.as_deref(),
        drop_params: scenario.drop_params,
    })?;
    Ok(serde_json::to_value(&request).unwrap())
}

fn batch(scenario: &Scenario, request: &Value) -> Result<Vec<MarengoBilledRequest>, String> {
    if !scenario.batch.is_empty() {
        return scenario
            .batch
            .iter()
            .map(|entry| serde_json::from_value(entry.clone()).map_err(|error| error.to_string()))
            .collect();
    }
    serde_json::from_value(request.clone())
        .map(|parsed| vec![parsed])
        .map_err(|error| error.to_string())
}

fn responses(scenario: &Scenario) -> Result<Vec<MarengoInvokeResponse>, String> {
    scenario
        .replies
        .iter()
        .map(|reply| serde_json::from_value(reply.clone()).map_err(|error| error.to_string()))
        .collect()
}

fn expected_value(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn verify(scenario: &Scenario, check: &Check) {
    match check {
        Check::Supported { marengo_3 } => {
            let supported = supported_openai_params(scenario.model.as_deref());
            let expected: Vec<&str> = if *marengo_3 {
                MARENGO_SHARED_PARAMS.to_vec()
            } else {
                MARENGO_SHARED_PARAMS
                    .iter()
                    .chain(MARENGO_2_7_ONLY_PARAMS.iter())
                    .copied()
                    .collect()
            };
            assert_eq!(supported, expected);
        }
        Check::Mapped(expected) => {
            let mapped = map_openai_params(
                is_marengo_3_model(scenario.model.as_deref()),
                &scenario.non_default,
                scenario.optional.clone(),
            );
            assert_eq!(serde_json::to_value(mapped).unwrap(), expected_value(expected));
        }
        Check::Request(expected) => {
            let built = transform(scenario).unwrap();
            assert_eq!(built, expected_value(expected));
        }
        Check::RequestError(tag) => match transform(scenario) {
            Ok(built) => panic!("expected {tag}, built {built:?}"),
            Err(MarengoEmbeddingsError::AsyncRouteRequired { .. }) => {
                assert_eq!(*tag, "async_route");
            }
            Err(MarengoEmbeddingsError::EmptyOutputS3Uri) => assert_eq!(*tag, "empty_output"),
            Err(MarengoEmbeddingsError::Marengo3(Marengo3Error::Provider { status, .. })) => {
                assert_eq!(*tag, "marengo3");
                assert_eq!(status, 400_u16);
            }
            Err(MarengoEmbeddingsError::Marengo3(Marengo3Error::Header(_))) => {
                panic!("unexpected header error")
            }
            Err(MarengoEmbeddingsError::Parameters { .. }) => assert_eq!(*tag, "parameters"),
            Err(MarengoEmbeddingsError::UsageOverflow) => panic!("usage overflow"),
        },
        Check::Response(expected) => {
            let request = transform(scenario).unwrap();
            let response = if async_branch(scenario) {
                let reply: Params =
                    serde_json::from_value(scenario.replies[0].clone()).unwrap();
                transform_async_invoke_response(&reply, scenario.model.as_deref().unwrap_or_default())
            } else {
                let parsed_batch = batch(scenario, &request).unwrap();
                let parsed = responses(scenario).unwrap();
                transform_response(
                    &parsed,
                    scenario.model.as_deref().unwrap_or_default(),
                    Some(&parsed_batch),
                )
                .unwrap()
            };
            assert_eq!(serde_json::to_value(&response).unwrap(), expected_value(expected));
        }
        Check::ReplyError => {
            assert!(responses(scenario).is_err());
        }
        Check::BatchError => {
            let request = transform(scenario).unwrap();
            assert!(batch(scenario, &request).is_err());
        }
        Check::Usage(expected) => {
            let request = transform(scenario).unwrap();
            let parsed_batch = batch(scenario, &request).unwrap();
            let usage = billed_usage(Some(&parsed_batch)).unwrap();
            assert_eq!(serde_json::to_value(usage).unwrap(), expected_value(expected));
        }
        Check::Enabled(expected) => {
            let policy = DropPolicy {
                drop_params: scenario.deployment_drop_params,
                litellm_drop_params: scenario.global_drop_params,
            };
            assert_eq!(drop_params_enabled(policy), *expected);
        }
    }
}

fn async_branch(scenario: &Scenario) -> bool {
    scenario.async_invoke_route
        && scenario.replies.len() == 1
        && scenario.replies[0].get("invocationArn").is_some()
}

const TEXT_BODY: &str = r#"{"inputType":"text","inputText":"hello","textTruncate":"end"}"#;

const IMAGE_BODY: &str =
    r#"{"inputType":"image","mediaSource":{"base64String":"hello"}}"#;

const WRAPPED_VIDEO: &str = r#"{
    "modelId": "twelvelabs.marengo-embed-2-7-v1:0",
    "modelInput": {
        "inputType": "video",
        "mediaSource": {"s3Location": {"uri": "s3://bucket/clip.mp4", "bucketOwner": "123456789012"}},
        "startSec": 1.5
    },
    "outputDataConfig": {"s3OutputDataConfig": {"s3Uri": "s3://out/bucket"}}
}"#;

#[rstest]
#[case::marengo_text_input(r#"{"model": "twelvelabs.marengo-embed-2-7-v1:0"}"#, Check::Supported { marengo_3: false })]
#[case::marengo_text_input(r#"{"model": "amazon.marengo-embed-3-0-v1:0"}"#, Check::Supported { marengo_3: true })]
#[case::marengo_text_input(r#"{"model": "TwelveLabs.Marengo-Embed-3-0-v1:0"}"#, Check::Supported { marengo_3: false })]
#[case::marengo_text_input(
    r#"{"model": "twelvelabs.marengo-embed-2-7-v1:0", "non_default": {"encoding_format": "float"}}"#,
    Check::Mapped(r#"{"embeddingOption": ["visual-text", "visual-image"]}"#)
)]
#[case::marengo_text_input(
    r#"{"model": "twelvelabs.marengo-embed-2-7-v1:0", "non_default": {"encoding_format": "base64"}}"#,
    Check::Mapped("{}")
)]
#[case::marengo_text_input(
    r#"{"model": "amazon.marengo-embed-3-0-v1:0", "non_default": {"encoding_format": "float"}}"#,
    Check::Mapped("{}")
)]
#[case::marengo_text_input(
    r#"{"non_default": {"encoding_format": "float", "embeddingOption": ["audio"]}}"#,
    Check::Mapped(r#"{"embeddingOption": ["audio"]}"#)
)]
#[case::marengo_text_input(
    r#"{"non_default": {
        "input_type": "video",
        "startSec": 1,
        "lengthSec": 2,
        "textTruncate": "none",
        "segmentation": {"method": "fixed", "fixed": {"durationSec": 4}},
        "endSec": 3,
        "embeddingType": ["fused_embedding"],
        "embeddingScope": ["clip"],
        "inferenceId": "x",
        "media_source": "m",
        "media_sources": {"a": "b"},
        "unknown": 1
    }}"#,
    Check::Mapped(r#"{
        "inputType": "video",
        "startSec": 1,
        "lengthSec": 2,
        "textTruncate": "none",
        "segmentation": {"method": "fixed", "fixed": {"durationSec": 4}},
        "endSec": 3,
        "embeddingType": ["fused_embedding"],
        "embeddingScope": ["clip"],
        "inferenceId": "x",
        "media_source": "m",
        "media_sources": {"a": "b"}
    }"#)
)]
#[case::marengo_text_input(r#"{"input": "hello"}"#, Check::Request(TEXT_BODY))]
#[case::marengo_text_input(
    r#"{"input": "hello", "inference": {"textTruncate": "none"}}"#,
    Check::Request(r#"{"inputType": "text", "inputText": "hello", "textTruncate": "none"}"#)
)]
#[case::marengo_text_input(
    r#"{"input": "hello", "inference": {"textTruncate": null}}"#,
    Check::Request(r#"{"inputType": "text", "inputText": "hello", "textTruncate": null}"#)
)]
#[case::marengo_text_input(
    r#"{"model": "amazon.marengo-embed-3-0-v1:0", "input": "hello"}"#,
    Check::Request(r#"{"inputType": "text", "text": {"inputText": "hello"}}"#)
)]
#[case::marengo_text_input(
    r#"{"input": "hello", "inference": {"input_type": "image"}}"#,
    Check::Request(IMAGE_BODY)
)]
#[case::marengo_s3_video_owner(
    r#"{"input": "s3://bucket/clip.mp4", "inference": {"inputType": "video"}}"#,
    Check::RequestError("async_route")
)]
#[case::marengo_s3_video_owner(
    r#"{"input": "s3://bucket/clip.mp3", "inference": {"input_type": "audio"}}"#,
    Check::RequestError("async_route")
)]
#[case::marengo_s3_video_owner(
    r#"{
        "model": "twelvelabs.marengo-embed-2-7-v1:0",
        "input": "s3://bucket/clip.mp4",
        "inference": {"inputType": "video", "bucketOwner": "123456789012", "startSec": 1.5},
        "async_invoke_route": true,
        "model_id": "async_invoke/twelvelabs.marengo-embed-2-7-v1:0",
        "output_s3_uri": "s3://out/bucket"
    }"#,
    Check::Request(WRAPPED_VIDEO)
)]
#[case::marengo_s3_video_owner(
    r#"{"input": "s3://bucket/key.png", "inference": {"inputType": "image"}}"#,
    Check::Request(r#"{"inputType": "image", "mediaSource": {"s3Location": {"uri": "s3://bucket/key.png"}}}"#)
)]
#[case::marengo_s3_video_owner(
    r#"{"input": "s3://bucket/key.png", "inference": {"inputType": "image", "bucketOwner": ""}}"#,
    Check::Request(r#"{"inputType": "image", "mediaSource": {"s3Location": {"uri": "s3://bucket/key.png"}}}"#)
)]
#[case::marengo_s3_video_owner(
    r#"{"input": "s3://bucket/key.png", "inference": {"inputType": "image", "bucketOwner": "123"}}"#,
    Check::Request(r#"{"inputType": "image", "mediaSource": {"s3Location": {"uri": "s3://bucket/key.png", "bucketOwner": "123"}}}"#)
)]
#[case::marengo_s3_video_owner(
    r#"{"input": "hello", "inference": {"inputType": "bogus"}}"#,
    Check::Request(r#"{"inputType": "bogus"}"#)
)]
#[case::marengo_s3_video_owner(
    r#"{
        "model": "twelvelabs.marengo-embed-2-7-v1:0",
        "input": "s3://bucket/clip.mp4",
        "inference": {"inputType": "video", "bucketOwner": "123456789012"},
        "async_invoke_route": true,
        "model_id": "twelvelabs.marengo-embed-2-7-v1:0",
        "output_s3_uri": "s3://out/bucket",
        "replies": [{"invocationArn": "arn:aws:bedrock:us-east-1:1:async-invoke/abc"}]
    }"#,
    Check::Response(r#"{
        "data": [{"embedding": [], "index": 0, "object": "embedding"}],
        "model": "twelvelabs.marengo-embed-2-7-v1:0",
        "usage": {"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0, "prompt_tokens_details": null},
        "invocation_arn": "arn:aws:bedrock:us-east-1:1:async-invoke/abc"
    }"#)
)]
#[case::marengo_image_input(
    r#"{"input": "data:image/png;base64,AAA", "inference": {"inputType": "image"}}"#,
    Check::Request(r#"{"inputType": "image", "mediaSource": {"base64String": "AAA"}}"#)
)]
#[case::marengo_image_input(
    r#"{"input": "data:image/png;base64,AAA,BBB", "inference": {"inputType": "image"}}"#,
    Check::Request(r#"{"inputType": "image", "mediaSource": {"base64String": "AAA,BBB"}}"#)
)]
#[case::marengo_image_input(
    r#"{"input": "data:image/png", "inference": {"inputType": "image"}}"#,
    Check::Request(r#"{"inputType": "image", "mediaSource": {"base64String": "data:image/png"}}"#)
)]
#[case::marengo_image_input(
    r#"{"input": "AAA,BBB,CCC", "inference": {"inputType": "image"}}"#,
    Check::Request(r#"{"inputType": "image", "mediaSource": {"base64String": "BBB"}}"#)
)]
#[case::marengo_image_input(
    r#"{"input": "AAA", "inference": {"inputType": "image"}}"#,
    Check::Request(IMAGE_BODY)
)]
#[case::marengo_image_input(
    r#"{"input": "AAA", "inference": {
        "inputType": "image",
        "inputText": "x",
        "mediaSource": "y",
        "bucketOwner": "z",
        "output_s3_uri": "s3://out"
    }}"#,
    Check::Request(r#"{"inputType": "image", "mediaSource": {"base64String": "AAA"}, "output_s3_uri": "s3://out"}"#)
)]
#[case::marengo_image_input(
    r#"{"model": "amazon.marengo-embed-3-0-v1:0", "input": "data:image/png;base64,AAA", "inference": {"inputType": "image"}}"#,
    Check::Request(r#"{"inputType": "image", "image": {"mediaSource": {"base64String": "AAA"}}}"#)
)]
#[case::marengo_async_output_wrapper(
    r#"{"input": "hello", "async_invoke_route": true, "output_s3_uri": "s3://out"}"#,
    Check::Request(TEXT_BODY)
)]
#[case::marengo_async_output_wrapper(
    r#"{"input": "hello", "model_id": "twelvelabs.model", "output_s3_uri": "s3://out"}"#,
    Check::Request(TEXT_BODY)
)]
#[case::marengo_async_output_wrapper(
    r#"{"model": "twelvelabs.marengo-embed-2-7-v1:0", "input": "hello", "async_invoke_route": true, "model_id": "async_invoke%2Ftwelvelabs.marengo%3A0", "output_s3_uri": "s3://out"}"#,
    Check::Request(r#"{
        "modelId": "twelvelabs.marengo:0",
        "modelInput": {"inputType": "text", "inputText": "hello", "textTruncate": "end"},
        "outputDataConfig": {"s3OutputDataConfig": {"s3Uri": "s3://out"}}
    }"#)
)]
#[case::marengo_async_output_wrapper(
    r#"{"model": "twelvelabs.marengo-embed-2-7-v1:0", "input": "hello", "async_invoke_route": true, "model_id": "async_invoke/a/async_invoke/b", "output_s3_uri": "s3://out"}"#,
    Check::Request(r#"{
        "modelId": "a/b",
        "modelInput": {"inputType": "text", "inputText": "hello", "textTruncate": "end"},
        "outputDataConfig": {"s3OutputDataConfig": {"s3Uri": "s3://out"}}
    }"#)
)]
#[case::marengo_async_output_wrapper(
    r#"{"model": "twelvelabs.marengo-embed-2-7-v1:0", "input": "hello", "async_invoke_route": true, "model_id": "plain/model", "output_s3_uri": " s3://out "}"#,
    Check::Request(r#"{
        "modelId": "plain/model",
        "modelInput": {"inputType": "text", "inputText": "hello", "textTruncate": "end"},
        "outputDataConfig": {"s3OutputDataConfig": {"s3Uri": " s3://out "}}
    }"#)
)]
#[case::marengo_async_output_wrapper(
    r#"{"input": "hello", "async_invoke_route": true, "model_id": "twelvelabs.model"}"#,
    Check::RequestError("empty_output")
)]
#[case::marengo_async_output_wrapper(
    r#"{"input": "hello", "async_invoke_route": true, "model_id": "twelvelabs.model", "output_s3_uri": ""}"#,
    Check::RequestError("empty_output")
)]
#[case::marengo_async_output_wrapper(
    r#"{"input": "hello", "async_invoke_route": true, "model_id": "twelvelabs.model", "output_s3_uri": "   "}"#,
    Check::RequestError("empty_output")
)]
#[case::marengo_async_output_wrapper(
    r#"{
        "model": "twelvelabs.marengo-embed-2-7-v1:0",
        "input": "hello",
        "async_invoke_route": true,
        "model_id": "twelvelabs.marengo-embed-2-7-v1:0",
        "output_s3_uri": "s3://out",
        "replies": [{"invocationArn": null}]
    }"#,
    Check::Response(r#"{
        "data": [{"embedding": [], "index": 0, "object": "embedding"}],
        "model": "twelvelabs.marengo-embed-2-7-v1:0",
        "usage": {"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0, "prompt_tokens_details": null},
        "invocation_arn": null
    }"#)
)]
#[case::marengo_segments_vector_list(
    r#"{"input": "one", "replies": [{"data": [{"embedding": null}], "embedding": [9.0], "embeddings": [{"embedding": [8.0]}]}]}"#,
    Check::Response(r#"{
        "data": [],
        "model": "",
        "usage": {"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0, "prompt_tokens_details": {"query_count": 1, "image_count": null}},
        "invocation_arn": null
    }"#)
)]
#[case::marengo_segments_vector_list(
    r#"{"input": "one", "replies": [{"embedding": [], "embeddings": [{"embedding": [8.0]}]}]}"#,
    Check::Response(r#"{
        "data": [{"embedding": [], "index": 0, "object": "embedding"}],
        "model": "",
        "usage": {"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0, "prompt_tokens_details": {"query_count": 1, "image_count": null}},
        "invocation_arn": null
    }"#)
)]
#[case::marengo_segments_vector_list(
    r#"{"input": "one", "replies": [{"embeddings": [{"embedding": [3.5]}, {"embedding": null}, {"embedding": [4.0]}]}]}"#,
    Check::Response(r#"{
        "data": [
            {"embedding": [3.5], "index": 0, "object": "embedding"},
            {"embedding": [4.0], "index": 1, "object": "embedding"}
        ],
        "model": "",
        "usage": {"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0, "prompt_tokens_details": {"query_count": 1, "image_count": null}},
        "invocation_arn": null
    }"#)
)]
#[case::marengo_segments_vector_list(
    r#"{"input": "one", "replies": [{"data": [{"embedding": [1, true, "2.5"]}]}, {"data": [{"embedding": [3.0]}]}]}"#,
    Check::Response(r#"{
        "data": [
            {"embedding": [1.0, 1.0, 2.5], "index": 0, "object": "embedding"},
            {"embedding": [3.0], "index": 1, "object": "embedding"}
        ],
        "model": "",
        "usage": {"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0, "prompt_tokens_details": {"query_count": 1, "image_count": null}},
        "invocation_arn": null
    }"#)
)]
#[case::marengo_segments_vector_list(
    r#"{"input": "one", "replies": [{"data": [{"embedding": ["abc"]}]}]}"#,
    Check::ReplyError
)]
#[case::marengo_billed_units(
    r#"{"input": "one", "batch": [{"inputType": "text"}]}"#,
    Check::Usage(r#"{"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0, "prompt_tokens_details": {"query_count": 1, "image_count": null}}"#)
)]
#[case::marengo_billed_units(
    r#"{"input": "one", "batch": [{"inputType": "image"}]}"#,
    Check::Usage(r#"{"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0, "prompt_tokens_details": {"query_count": null, "image_count": 1}}"#)
)]
#[case::marengo_billed_units(
    r#"{"input": "one", "batch": [{"inputType": "text_image"}]}"#,
    Check::Usage(r#"{"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0, "prompt_tokens_details": {"query_count": 1, "image_count": 1}}"#)
)]
#[case::marengo_billed_units(
    r#"{"input": "one", "batch": [{"inputType": "video"}, {"inputType": "audio"}, {}]}"#,
    Check::Usage(r#"{"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0, "prompt_tokens_details": null}"#)
)]
#[case::marengo_billed_units(
    r#"{"input": "one", "batch": [{"inputType": "multi_input", "multi_input": {"inputText": "x", "mediaSources": [{}, {}]}}]}"#,
    Check::Usage(r#"{"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0, "prompt_tokens_details": {"query_count": 1, "image_count": 2}}"#)
)]
#[case::marengo_billed_units(
    r#"{"input": "one", "batch": [{"inputType": "multi_input", "multi_input": {"inputText": "", "mediaSources": [{}, {}]}}]}"#,
    Check::Usage(r#"{"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0, "prompt_tokens_details": {"query_count": null, "image_count": 2}}"#)
)]
#[case::marengo_billed_units(
    r#"{"input": "one", "batch": [{"inputType": "multi_input", "multi_input": {"inputText": "x"}}]}"#,
    Check::Usage(r#"{"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0, "prompt_tokens_details": {"query_count": 1, "image_count": null}}"#)
)]
#[case::marengo_billed_units(
    r#"{"input": "one", "replies": [{"data": [{"embedding": [1.0]}, {"embedding": [2.0]}, {"embedding": [3.0]}]}]}"#,
    Check::Response(r#"{
        "data": [
            {"embedding": [1.0], "index": 0, "object": "embedding"},
            {"embedding": [2.0], "index": 1, "object": "embedding"},
            {"embedding": [3.0], "index": 2, "object": "embedding"}
        ],
        "model": "",
        "usage": {"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0, "prompt_tokens_details": {"query_count": 1, "image_count": null}},
        "invocation_arn": null
    }"#)
)]
#[case::marengo_billed_units(
    r#"{"input": "one", "batch": [{"inputType": "bogus"}]}"#,
    Check::BatchError
)]
#[case::marengo_billed_units(
    r#"{"input": "one", "batch": [{"inputType": "multi_input", "multi_input": {"inputText": 1}}]}"#,
    Check::BatchError
)]
#[case::drop_unsupported_param(
    r#"{"global_drop_params": true}"#,
    Check::Enabled(true)
)]
#[case::drop_unsupported_param(
    r#"{"global_drop_params": false}"#,
    Check::Enabled(false)
)]
#[case::drop_unsupported_param(
    r#"{"deployment_drop_params": true}"#,
    Check::Enabled(true)
)]
#[case::drop_unsupported_param(
    r#"{"deployment_drop_params": false}"#,
    Check::Enabled(false)
)]
fn marengo_embeddings_cases(#[case] scenario: &str, #[case] check: Check) {
    let scenario: Scenario = serde_json::from_str(scenario).unwrap();
    verify(&scenario, &check);
}
