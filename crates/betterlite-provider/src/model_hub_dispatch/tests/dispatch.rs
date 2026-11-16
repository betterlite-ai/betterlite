use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_prompts::arguments::json::Object;

use super::super::{HubDispatchError, HubProvider, polling, select};
use super::support::{Services, chat_context, empty_globals, environment, run_with, user_message};
use crate::replicate::poll::ReplicatePolling;

#[test]
fn select_maps_only_the_three_hub_tags() {
    let hugging_face = select(ChatConfigTag::HuggingFaceChat);
    assert_eq!(hugging_face, Some(HubProvider::HuggingFace));
    assert_eq!(
        select(ChatConfigTag::Replicate),
        Some(HubProvider::Replicate)
    );
    assert_eq!(
        select(ChatConfigTag::Predibase),
        Some(HubProvider::Predibase)
    );
    assert_eq!(select(ChatConfigTag::OpenAiGpt), None);
    assert_eq!(select(ChatConfigTag::Anthropic), None);
}

#[tokio::test]
async fn dispatch_leaves_an_unowned_tag_alone() {
    let services = Services::new(Vec::new(), Vec::new(), Vec::new());
    let mut messages = vec![user_message()];
    let mut options = Object::new();
    let mut call = chat_context("m", &mut messages, &mut options, None);
    let arm = (ChatConfigTag::OpenAiGpt, &mut call);
    let (outcome, state) = run_with(&services, &empty_globals(), &environment(&[]), arm).await;
    assert!(outcome.unwrap().is_none());
    assert!(services.http.sent("POST").is_empty());
    assert!(services.transport.sent.lock().unwrap().is_empty());
    assert!(state.model_call_details.get("log_event_type").is_none());
}

#[test]
fn polling_reads_the_replicate_variables_and_reports_a_bad_one() {
    let env = environment(&[
        ("DEFAULT_REPLICATE_POLLING_RETRIES", "2"),
        ("DEFAULT_REPLICATE_POLLING_DELAY_SECONDS", "4"),
        ("REPLICATE_POLLING_DELAY_SECONDS", "0.25"),
        ("REPLICATE_MODEL_NAME_WITH_ID_LENGTH", "8"),
    ]);
    let found = polling(&env).unwrap();
    assert_eq!(found.retries, 2_i32);
    assert_eq!(found.retry_delay_seconds.to_bits(), 4.0_f64.to_bits());
    assert_eq!(found.polling_delay_seconds.to_bits(), 0.25_f64.to_bits());
    assert_eq!(found.model_name_with_id_length, 8);
    let defaults: ReplicatePolling = polling(&environment(&[])).unwrap();
    assert_eq!(defaults.retries, 5_i32);
    let bad = environment(&[("DEFAULT_REPLICATE_POLLING_RETRIES", "five")]);
    let error = polling(&bad).expect_err("a text retry count is an environment error");
    let HubDispatchError::Environment { source } = error else {
        panic!("a bad polling value is an environment error");
    };
    assert!(
        source
            .to_string()
            .contains("DEFAULT_REPLICATE_POLLING_RETRIES")
    );
}
