#![cfg(test)]

use crate::api::skills::fixtures::{
    CALL_ID, Fixture, SKILL_ID, call, call_with, hosted_params, kwargs, logging, skill_request,
    text,
};

#[tokio::test]
async fn pre_call_logging_records_skill_id_and_provider() {
    let fixture = Fixture::new();
    let values = kwargs();
    let mut state = logging();
    fixture
        .service()
        .get_skill(skill_request(&call(SKILL_ID, &values), &mut state))
        .await
        .expect("the external get succeeds");
    assert_eq!(state.optional_params.get("skill_id"), Some(&text(SKILL_ID)));
    assert_eq!(state.custom_llm_provider.as_deref(), Some("anthropic"));
    assert_eq!(
        state.model_call_details.get("skill_id"),
        Some(&text(SKILL_ID))
    );
}

#[tokio::test]
async fn logging_records_the_call_id_from_the_call() {
    let fixture = Fixture::new();
    let values = kwargs();
    let mut state = logging();
    fixture
        .service()
        .get_skill(skill_request(&call(SKILL_ID, &values), &mut state))
        .await
        .expect("the external get succeeds");
    assert_eq!(
        state.litellm_params.get("litellm_call_id"),
        Some(&text(CALL_ID))
    );
}

#[tokio::test]
async fn pre_call_logging_uses_litellm_proxy_for_hosted() {
    let fixture = Fixture::new();
    let values = kwargs();
    let hosted = call_with(hosted_params(), SKILL_ID, &values);
    let mut state = logging();
    fixture
        .service()
        .get_skill(skill_request(&hosted, &mut state))
        .await
        .expect("the hosted get succeeds");
    assert_eq!(state.custom_llm_provider.as_deref(), Some("litellm_proxy"));
    assert_eq!(state.optional_params.get("skill_id"), Some(&text(SKILL_ID)));
}
