#![cfg(test)]

use betterlite_roi::estimator::models::{RESPONSE_CONTRACT, ROICompletionRole, ReasoningEffort};
use serde_json::json;

use crate::roi_estimator::Estimator;

use super::support::{
    EVIDENCE, ModelSupport, OK_CONTENT, caller, ok_response, pull, recorded_request, settings,
};

#[tokio::test]
async fn estimate_request_carries_the_exact_prompt_messages_and_sampling() {
    let (complete, requests) = caller(Ok(ok_response(OK_CONTENT)));
    let estimator = Estimator::new(
        settings("gpt-5.2", "Estimate this pull request."),
        complete,
        Some(&[]),
    );
    estimator
        .estimate(&pull(), &mut ModelSupport::none())
        .await
        .unwrap();
    let request = recorded_request(&requests);
    assert_eq!(request.model, "gpt-5.2");
    assert_eq!(request.temperature, 0);
    assert_eq!(request.max_tokens, 1200);
    assert_eq!(request.messages[0].role, ROICompletionRole::System);
    assert_eq!(
        request.messages[0].content,
        format!("Estimate this pull request.\n\n{RESPONSE_CONTRACT}")
    );
    assert_eq!(request.messages[1].role, ROICompletionRole::User);
    assert_eq!(request.messages[1].content, EVIDENCE);
    assert_eq!(
        serde_json::to_value(request.response_format).unwrap(),
        json!({"type": "json_object"})
    );
    assert_eq!(request.reasoning_effort, None);
    assert_eq!(
        serde_json::to_value(&request)
            .unwrap()
            .get("reasoning_effort"),
        Some(&serde_json::Value::Null)
    );
}

#[tokio::test]
async fn estimate_request_sends_reasoning_effort_none_when_every_model_supports_it() {
    let (complete, requests) = caller(Ok(ok_response(OK_CONTENT)));
    let estimator = Estimator::new(settings("gpt-5.2", "Estimate this."), complete, None);
    let mut predicates = ModelSupport::supporting(&["gpt-5.2"]);
    estimator.estimate(&pull(), &mut predicates).await.unwrap();
    let request = recorded_request(&requests);
    assert_eq!(request.reasoning_effort, Some(ReasoningEffort::None));
    assert_eq!(
        serde_json::to_value(&request)
            .unwrap()
            .get("reasoning_effort"),
        Some(&json!("none"))
    );
}
