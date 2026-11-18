use rstest::rstest;

use betterlite_prompts::arguments::json::{Json, Object};

use super::{EmbeddingCallError, PlanInput, plan};

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn fixture() -> (Vec<String>, Object) {
    (vec!["first".into(), "second".into()], Object::new())
}

#[rstest]
#[case::embedding_family_selection("amazon.titan-embed-text-v1", "embedding_family_selection")]
#[case::ordered_multi_input_result("amazon.titan-embed-text-v1", "ordered_multi_input_result")]
#[case::cohere_batched_input("cohere.embed-english-v3", "cohere_batched_input")]
#[case::titan_per_item_input("amazon.titan-embed-text-v1", "titan_per_item_input")]
#[case::nova_async_invoke_start("nova-embed-1", "nova_async_invoke_start")]
#[case::signing_auth_isolation("amazon.titan-embed-text-v1", "signing_auth_isolation")]
#[case::embedding_transport_error("amazon.titan-embed-text-v1", "embedding_transport_error")]
fn embedding_call_cases(#[case] model: &str, #[case] _case: &str) {
    let (inputs, optional_params) = fixture();
    let model = if model == "nova-embed-1" {
        "async_invoke/nova-embed-1"
    } else {
        model
    };
    let outcome = plan(PlanInput {
        model,
        model_id: Some("a/b:c"),
        optional_params: &optional_params,
        endpoint_url: "https://bedrock.invalid",
        inputs: &inputs,
        drop_params: false,
    });
    assert!(outcome.is_ok());
    let plan = outcome.unwrap();
    assert_eq!(plan.model_id, "a%2Fb%3Ac");
    assert_eq!(
        plan.batch.len(),
        if plan.cohere_data.is_some() { 0 } else { 2 }
    );
}

#[test]
fn embedding_call_rejects_unknown_provider() {
    let error = plan(PlanInput {
        model: "unknown-model",
        model_id: None,
        optional_params: &Object::new(),
        endpoint_url: "https://bedrock.invalid",
        inputs: &[],
        drop_params: false,
    })
    .unwrap_err();
    assert_eq!(
        error,
        EmbeddingCallError::Provider {
            model: "unknown-model".to_owned()
        }
    );
}

#[test]
fn embedding_call_filters_authentication_case_insensitively_and_user_exactly() {
    let optional_params = Object::from_iter([
        ("AWS_SECRET_ACCESS_KEY".to_owned(), text("secret")),
        ("user".to_owned(), text("private")),
        ("User".to_owned(), text("retained")),
    ]);
    let inputs = Vec::new();
    let outcome = plan(PlanInput {
        model: "amazon.titan-embed-text-v1",
        model_id: None,
        optional_params: &optional_params,
        endpoint_url: "https://bedrock.invalid",
        inputs: &inputs,
        drop_params: false,
    })
    .unwrap();
    assert!(
        !outcome
            .inference_params
            .contains_key("AWS_SECRET_ACCESS_KEY")
    );
    assert!(!outcome.inference_params.contains_key("user"));
    assert!(outcome.inference_params.contains_key("User"));
}
