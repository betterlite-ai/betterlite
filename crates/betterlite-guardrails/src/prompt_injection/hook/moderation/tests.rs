#![cfg(test)]

use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_config::sections::prompt_injection::PromptInjectionParams;
use betterlite_models::billing_text::FormattedPromptData;
use reqwest::StatusCode;

use super::hook_support::{
    ATTACK, FAIL_CALL_STRING, JUDGE_MODEL, SYSTEM_PROMPT, Scenario, chat_messages, params, text,
    threshold, timeout,
};
use crate::prompt_injection::error::PromptInjectionError;
use crate::prompt_injection::hook::{HookCallType, InjectionJudge, PromptInjectionDetection};
use crate::prompt_injection::pool::HeuristicsPool;

enum Reply {
    Text(String),
    Nothing,
    Failure,
}

struct ScriptedJudge {
    reply: Reply,
    calls: Mutex<Vec<(String, String, String)>>,
}

impl ScriptedJudge {
    fn new(reply: Reply) -> Self {
        Self {
            reply,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<(String, String, String)> {
        self.calls.lock().expect("the judge calls lock").clone()
    }
}

#[async_trait]
impl InjectionJudge for ScriptedJudge {
    async fn complete(
        &self,
        model: &str,
        system_prompt: &str,
        user_prompt: &str,
    ) -> Result<Option<String>, PromptInjectionError> {
        self.calls.lock().expect("the judge calls lock").push((
            model.to_owned(),
            system_prompt.to_owned(),
            user_prompt.to_owned(),
        ));
        match &self.reply {
            Reply::Text(answer) => Ok(Some(answer.clone())),
            Reply::Nothing => Ok(None),
            Reply::Failure => Err(PromptInjectionError::Judge),
        }
    }
}

fn detection(params: Option<PromptInjectionParams>) -> PromptInjectionDetection {
    PromptInjectionDetection::new(params, threshold(), HeuristicsPool::new(1, timeout()))
}

fn detection_with_names(
    params: PromptInjectionParams,
    names: &[String],
) -> PromptInjectionDetection {
    let mut detection = detection(Some(params));
    detection
        .update_environment(Some(names))
        .expect("the configured model name is in the list");
    detection
}

fn listed_names() -> [String; 1] {
    [JUDGE_MODEL.to_owned()]
}

#[tokio::test]
async fn without_params_moderation_returns_none() {
    let judge = ScriptedJudge::new(Reply::Text(FAIL_CALL_STRING.to_owned()));
    let detection = detection(None);
    let list = chat_messages();
    let request = FormattedPromptData {
        messages: Some(&list),
        prompt: None,
        input: None,
    };
    let outcome = detection
        .moderation(&judge, HookCallType::Completion, &request)
        .await
        .unwrap();
    assert_eq!(outcome, None);
    assert!(judge.calls().is_empty());
}

#[tokio::test]
async fn an_empty_prompt_returns_none() {
    let judge = ScriptedJudge::new(Reply::Text(FAIL_CALL_STRING.to_owned()));
    let detection = detection_with_names(params(Scenario::LlmApi), &listed_names());
    let request = FormattedPromptData {
        messages: None,
        prompt: None,
        input: None,
    };
    let outcome = detection
        .moderation(&judge, HookCallType::AudioTranscription, &request)
        .await
        .unwrap();
    assert_eq!(outcome, None);
    assert!(judge.calls().is_empty());
}

#[tokio::test]
async fn embeddings_have_no_prompt_to_judge() {
    let judge = ScriptedJudge::new(Reply::Text(FAIL_CALL_STRING.to_owned()));
    let detection = detection_with_names(params(Scenario::LlmApi), &listed_names());
    let input = text(ATTACK);
    let request = FormattedPromptData {
        messages: None,
        prompt: None,
        input: Some(&input),
    };
    let outcome = detection
        .moderation(&judge, HookCallType::Embeddings, &request)
        .await
        .unwrap();
    assert_eq!(outcome, None);
    assert!(judge.calls().is_empty());
}

#[tokio::test]
async fn a_disabled_llm_api_check_returns_some_false() {
    let judge = ScriptedJudge::new(Reply::Text(FAIL_CALL_STRING.to_owned()));
    let detection = detection(Some(params(Scenario::AllFalse)));
    let list = chat_messages();
    let request = FormattedPromptData {
        messages: Some(&list),
        prompt: None,
        input: None,
    };
    let outcome = detection
        .moderation(&judge, HookCallType::Completion, &request)
        .await
        .unwrap();
    assert_eq!(outcome, Some(false));
    assert!(judge.calls().is_empty());
}

#[tokio::test]
async fn a_judged_attack_is_rejected_and_the_call_is_captured() {
    let judge = ScriptedJudge::new(Reply::Text(format!("This is {FAIL_CALL_STRING}.")));
    let detection = detection_with_names(params(Scenario::LlmApi), &listed_names());
    let list = chat_messages();
    let request = FormattedPromptData {
        messages: Some(&list),
        prompt: None,
        input: None,
    };
    let outcome = detection
        .moderation(&judge, HookCallType::Completion, &request)
        .await;
    assert!(matches!(outcome, Err(PromptInjectionError::Rejected)));
    assert_eq!(
        judge.calls(),
        vec![(
            JUDGE_MODEL.to_owned(),
            SYSTEM_PROMPT.to_owned(),
            ATTACK.to_owned()
        )]
    );
}

#[tokio::test]
async fn reject_as_response_does_not_turn_a_judged_attack_into_a_message() {
    let judge = ScriptedJudge::new(Reply::Text(FAIL_CALL_STRING.to_owned()));
    let detection = detection_with_names(params(Scenario::LlmApiRejectAsResponse), &listed_names());
    let list = chat_messages();
    let request = FormattedPromptData {
        messages: Some(&list),
        prompt: None,
        input: None,
    };
    let error = detection
        .moderation(&judge, HookCallType::Completion, &request)
        .await
        .expect_err("a judged attack is rejected");
    assert!(matches!(error, PromptInjectionError::Rejected));
    let rejection = error.rejection().expect("a rejection carries a status");
    assert_eq!(rejection.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn the_fail_call_string_is_case_sensitive() {
    let judge = ScriptedJudge::new(Reply::Text("this is unsafe".to_owned()));
    let detection = detection_with_names(params(Scenario::LlmApi), &listed_names());
    let list = chat_messages();
    let request = FormattedPromptData {
        messages: Some(&list),
        prompt: None,
        input: None,
    };
    let outcome = detection
        .moderation(&judge, HookCallType::Completion, &request)
        .await
        .unwrap();
    assert_eq!(outcome, Some(false));
}

#[tokio::test]
async fn non_chat_answers_and_clean_answers_are_safe() {
    let list = chat_messages();
    let request = FormattedPromptData {
        messages: Some(&list),
        prompt: None,
        input: None,
    };
    let silent = ScriptedJudge::new(Reply::Nothing);
    let clean = ScriptedJudge::new(Reply::Text("SAFE".to_owned()));
    let detection = detection_with_names(params(Scenario::LlmApi), &listed_names());
    assert_eq!(
        detection
            .moderation(&silent, HookCallType::Completion, &request)
            .await
            .unwrap(),
        Some(false)
    );
    assert_eq!(
        detection
            .moderation(&clean, HookCallType::Completion, &request)
            .await
            .unwrap(),
        Some(false)
    );
}

#[tokio::test]
async fn judge_failures_propagate() {
    let judge = ScriptedJudge::new(Reply::Failure);
    let detection = detection_with_names(params(Scenario::LlmApi), &listed_names());
    let list = chat_messages();
    let request = FormattedPromptData {
        messages: Some(&list),
        prompt: None,
        input: None,
    };
    let outcome = detection
        .moderation(&judge, HookCallType::Completion, &request)
        .await;
    assert!(matches!(outcome, Err(PromptInjectionError::Judge)));
}

#[tokio::test]
async fn prompt_building_errors_propagate() {
    let judge = ScriptedJudge::new(Reply::Text(FAIL_CALL_STRING.to_owned()));
    let detection = detection_with_names(params(Scenario::LlmApi), &listed_names());
    let request = FormattedPromptData {
        messages: None,
        prompt: None,
        input: None,
    };
    let outcome = detection
        .moderation(&judge, HookCallType::Acompletion, &request)
        .await;
    assert!(matches!(outcome, Err(PromptInjectionError::Prompt { .. })));
}

#[tokio::test]
async fn a_missing_model_list_skips_the_judge() {
    let judge = ScriptedJudge::new(Reply::Text(FAIL_CALL_STRING.to_owned()));
    let mut detection = detection(Some(params(Scenario::LlmApi)));
    assert!(
        detection
            .update_environment(None)
            .expect_err("the model list is required")
            .to_string()
            .contains("Model List not set")
    );
    let list = chat_messages();
    let request = FormattedPromptData {
        messages: Some(&list),
        prompt: None,
        input: None,
    };
    let outcome = detection
        .moderation(&judge, HookCallType::Completion, &request)
        .await
        .unwrap();
    assert_eq!(outcome, Some(false));
    assert!(judge.calls().is_empty());
}

#[tokio::test]
async fn a_missing_model_name_skips_the_judge() {
    let judge = ScriptedJudge::new(Reply::Text(FAIL_CALL_STRING.to_owned()));
    let mut detection = detection(Some(params(Scenario::LlmApiWithoutName)));
    assert!(
        detection
            .update_environment(Some(&listed_names()))
            .expect_err("the model name must be in the list")
            .to_string()
            .contains("Invalid LLM API Name")
    );
    let list = chat_messages();
    let request = FormattedPromptData {
        messages: Some(&list),
        prompt: None,
        input: None,
    };
    let outcome = detection
        .moderation(&judge, HookCallType::Completion, &request)
        .await
        .unwrap();
    assert_eq!(outcome, Some(false));
    assert!(judge.calls().is_empty());
}

#[tokio::test]
async fn moderation_does_not_run_heuristics() {
    let judge = ScriptedJudge::new(Reply::Text(FAIL_CALL_STRING.to_owned()));
    let detection = detection(Some(params(Scenario::Heuristics)));
    let list = chat_messages();
    let request = FormattedPromptData {
        messages: Some(&list),
        prompt: None,
        input: None,
    };
    let outcome = detection
        .moderation(&judge, HookCallType::Completion, &request)
        .await
        .unwrap();
    assert_eq!(outcome, Some(false));
    assert!(judge.calls().is_empty());
}

#[tokio::test]
async fn a_missing_system_prompt_is_sent_empty() {
    let judge = ScriptedJudge::new(Reply::Text("SAFE".to_owned()));
    let detection =
        detection_with_names(params(Scenario::LlmApiWithoutSystemPrompt), &listed_names());
    let list = chat_messages();
    let request = FormattedPromptData {
        messages: Some(&list),
        prompt: None,
        input: None,
    };
    let outcome = detection
        .moderation(&judge, HookCallType::Completion, &request)
        .await
        .unwrap();
    assert_eq!(outcome, Some(false));
    assert_eq!(judge.calls().first().map(|call| call.1.as_str()), Some(""));
}
