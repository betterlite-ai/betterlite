use betterlite_openai::embedding::EmbeddingVector;
use betterlite_prompts::arguments::json::Object;

use super::super::fixture::{Probe, Reply, Run, object, run};
use super::super::support::{batch, titan_body, vector};
use crate::bedrock_embedding_io::response::PromptTokenCounts;
use crate::embedding_call::error::EmbeddingCallError;

pub(super) fn titan_per_item_input() {
    the_four_exact_ids_build_their_bodies();
    one_body_per_input_in_order();
    another_amazon_model_unmaps();
    response_model_ignores_model_id();
    v2_reads_the_binary_vector();
    multimodal_counts_images();
}

fn the_four_exact_ids_build_their_bodies() {
    for model in [
        "amazon.titan-embed-text-v1",
        "amazon.titan-embed-g1-text-02",
    ] {
        let outcome = with_dimensions(model);
        assert_eq!(outcome.body(), object("{\"inputText\":\"hi\"}"));
    }
    let v2 = with_dimensions("amazon.titan-embed-text-v2:0");
    assert_eq!(
        v2.body(),
        object("{\"inputText\":\"hi\",\"User\":\"bob\",\"dimensions\":256}")
    );
    let multimodal = with_dimensions("amazon.titan-embed-image-v1");
    assert_eq!(
        multimodal.body(),
        object("{\"inputText\":\"hi\",\"dimensions\":256,\"User\":\"bob\"}")
    );
}

fn with_dimensions(model: &str) -> Run {
    run(&Probe {
        model: model.to_owned(),
        input: vec![String::from("hi")],
        optional_params: vec![
            ("aws_access_key_id", "\"AKIAIOSFODNN7EXAMPLE\""),
            (
                "aws_secret_access_key",
                "\"wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY\"",
            ),
            ("aws_region_name", "\"us-east-1\""),
            ("dimensions", "256"),
            ("user", "\"alice\""),
            ("User", "\"bob\""),
        ],
        replies: vec![Reply::ok(&titan_body("[0.5]", 1))],
        ..Probe::default()
    })
}

fn sent_body(run: &Run, index: usize) -> Object {
    serde_json::from_slice(&run.factory_requests[index].body).expect("the sent body is an object")
}

fn one_body_per_input_in_order() {
    let outcome = run(&Probe {
        input: vec![String::from("a"), String::from("b")],
        replies: vec![
            Reply::ok(&titan_body("[1.0]", 2)),
            Reply::ok(&titan_body("[2.0]", 3)),
        ],
        ..Probe::default()
    });
    assert_eq!(outcome.factory_requests.len(), 2);
    assert_eq!(sent_body(&outcome, 0), object("{\"inputText\":\"a\"}"));
    assert_eq!(sent_body(&outcome, 1), object("{\"inputText\":\"b\"}"));
    assert_eq!(vector(&batch(outcome.response()).data[0].embedding), [1.0]);
}

fn another_amazon_model_unmaps() {
    let outcome = run(&Probe {
        model: String::from("amazon.titan-embed-text-v3"),
        optional_params: vec![
            ("aws_access_key_id", "\"AKIAIOSFODNN7EXAMPLE\""),
            (
                "aws_secret_access_key",
                "\"wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY\"",
            ),
            ("aws_region_name", "\"us-east-1\""),
            (
                "aws_bedrock_runtime_endpoint",
                "\"https://runtime.example.test\"",
            ),
        ],
        ..Probe::default()
    });
    assert!(matches!(outcome.error(), EmbeddingCallError::Unmapped));
    assert_eq!(
        outcome.error().to_string(),
        "Unable to map Bedrock request to provider"
    );
    assert!(outcome.factory_requests.is_empty());
    assert!(
        !outcome
            .remaining
            .contains_key("aws_bedrock_runtime_endpoint")
    );
}

fn response_model_ignores_model_id() {
    let outcome = run(&Probe {
        optional_params: vec![
            ("aws_access_key_id", "\"AKIAIOSFODNN7EXAMPLE\""),
            (
                "aws_secret_access_key",
                "\"wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY\"",
            ),
            ("aws_region_name", "\"us-east-1\""),
            ("model_id", "\"custom-model\""),
        ],
        replies: vec![Reply::ok(&titan_body("[0.5]", 1))],
        ..Probe::default()
    });
    assert_eq!(
        batch(outcome.response()).model,
        "amazon.titan-embed-text-v1"
    );
    assert_eq!(
        outcome.posted().url,
        "https://bedrock-runtime.us-east-1.amazonaws.com/model/custom-model/invoke"
    );
}

fn v2_reads_the_binary_vector() {
    let outcome = run(&Probe {
        model: String::from("amazon.titan-embed-text-v2:0"),
        replies: vec![Reply::ok(
            "{\"embeddingsByType\":{\"binary\":\"QUJD\"},\"inputTextTokenCount\":2}",
        )],
        ..Probe::default()
    });
    let reply = batch(outcome.response());
    match &reply.data[0].embedding {
        EmbeddingVector::Base64(encoded) => assert_eq!(encoded, "QUJD"),
        EmbeddingVector::Floats(_) => panic!("the binary reply keeps its encoded vector"),
    }
    assert_eq!(reply.usage.prompt_tokens, 2);
}

fn multimodal_counts_images() {
    let outcome = run(&Probe {
        model: String::from("amazon.titan-embed-image-v1"),
        input: vec![String::from("data:image/png;base64,AAAA")],
        replies: vec![Reply::ok(&titan_body("[0.5]", 1))],
        ..Probe::default()
    });
    assert_eq!(outcome.body(), object("{\"inputImage\":\"AAAA\"}"));
    assert_eq!(
        batch(outcome.response()).usage.prompt_tokens_details,
        Some(PromptTokenCounts {
            image_count: Some(1),
            query_count: None,
        })
    );
}
