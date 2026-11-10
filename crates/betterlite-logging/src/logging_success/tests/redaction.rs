use std::sync::Arc;

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::{
    CallLog, FakeLogger, Fixture, MessageFlags, chat_result, correlation, payload, request, state,
};
use crate::logging_success::dispatch::dispatch_success_handlers;

fn is_placeholder_message(messages: Option<&Json>) -> bool {
    let Some(Json::List(items)) = messages else {
        return false;
    };
    let Some(Json::Object(fields)) = items.first() else {
        return false;
    };
    fields.get("content") == Some(&Json::Text("redacted-by-litellm".to_owned()))
        && fields.get("role") == Some(&Json::Text("assistant".to_owned()))
}

fn is_placeholder_response(response: Option<&Json>) -> bool {
    let Some(Json::Object(fields)) = response else {
        return false;
    };
    if fields.get("id") != Some(&Json::Text("chatcmpl-placeholder".to_owned())) {
        return false;
    }
    match fields.get("choices") {
        Some(Json::List(choices)) => choices
            .first()
            .is_some_and(|choice| matches!(choice, Json::Object(_))),
        _ => false,
    }
}

#[rstest]
#[case::untouched("untouched")]
#[case::turn_off("turn_off")]
#[case::self_redacting("self_redacting")]
#[case::excluded_messages("excluded_messages")]
#[case::excluded_required_model("excluded_required_model")]
#[case::sync_turn_off("sync_turn_off")]
#[tokio::test]
async fn logging_success_behavior_tests_behavior(#[case] scenario: &str) {
    let mut fixture = Fixture::new();
    let calls = Arc::new(CallLog::default());
    let mut logger = FakeLogger::new("langfuse", &calls);
    logger.flags = MessageFlags {
        turn_off_message_logging: matches!(
            scenario,
            "turn_off" | "self_redacting" | "sync_turn_off"
        ),
        redacts_messages_itself: scenario == "self_redacting",
    };
    fixture.add(&Arc::new(logger));
    fixture.excluded_fields = match scenario {
        "excluded_messages" => vec!["messages".to_owned()],
        "excluded_required_model" => vec!["model".to_owned()],
        _ => Vec::new(),
    };
    let asynchronous = scenario != "sync_turn_off";
    let mut state = state(Some(false));
    if asynchronous {
        state
            .litellm_params
            .insert("acompletion".to_owned(), Json::Bool(true));
    }
    let mut correlation = correlation();
    dispatch_success_handlers(
        &fixture.handler(),
        &mut state,
        &mut correlation,
        &request(chat_result()),
    )
    .await;
    let call = calls.calls().into_iter().next().unwrap();
    match scenario {
        "untouched" | "sync_turn_off" | "self_redacting" => {
            assert_eq!(call.messages, payload().messages);
            assert_eq!(call.response, payload().response);
        }
        "turn_off" => {
            assert!(is_placeholder_message(call.messages.as_ref()));
            assert!(is_placeholder_response(call.response.as_ref()));
        }
        "excluded_messages" => {
            assert_eq!(call.messages, None);
            assert_eq!(call.response, payload().response);
        }
        "excluded_required_model" => {
            assert_eq!(call.model, "gpt-4o");
            assert_eq!(call.messages, payload().messages);
        }
        other => panic!("unexpected scenario {other}"),
    }
}
