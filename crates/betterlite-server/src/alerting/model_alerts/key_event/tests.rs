#![cfg(test)]

use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_logging::alert_dispatch::AlertLevel;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use super::{KeyEventAlerts, VirtualKeyEvent, render_key_event_message};
use crate::alerting::model_alerts::tests::support::{FakeSender, LogCapture};

fn kwargs(entries: Vec<(&str, Json)>) -> JsonObject {
    let mut kwargs = IndexMap::new();
    for (key, value) in entries {
        kwargs.insert(key.to_owned(), value);
    }
    kwargs
}

fn key_event(alias: Option<&str>, request_kwargs: JsonObject) -> VirtualKeyEvent {
    VirtualKeyEvent {
        created_by_user_id: "user-1".to_owned(),
        created_by_user_role: "proxy_admin".to_owned(),
        created_by_key_alias: alias.map(str::to_owned),
        request_kwargs,
    }
}

fn alerts(sender: FakeSender) -> KeyEventAlerts<FakeSender> {
    KeyEventAlerts { sender }
}

#[test]
fn renders_the_exact_key_event_message() {
    let event = key_event(
        None,
        kwargs(vec![
            ("user_api_key_dict", Json::Object(IndexMap::new())),
            ("key_alias", Json::Text("my-key".to_owned())),
            (
                "models",
                Json::List(vec![Json::Text("a".to_owned()), Json::Text("b".to_owned())]),
            ),
            ("max_budget", Json::Number(Number::from(10_u32))),
            ("blocked", Json::Bool(true)),
            ("team_id", Json::Null),
        ]),
    );

    let message =
        render_key_event_message(&event, "New Virtual Key Created").expect("the message renders");

    assert_eq!(
        message,
        "`New Virtual Key Created`\n*Action Done by:*\ncreated_by_user_id: `user-1`\ncreated_by_user_role: `proxy_admin`\ncreated_by_key_alias: `None`\n\n*Arguments passed:*\nkey_alias: `my-key`\nmodels: `[\"a\",\"b\"]`\nmax_budget: `10`\nblocked: `true`\nteam_id: `None`\n"
    );
}

#[test]
fn an_alias_is_printed_between_the_role_and_the_arguments() {
    let event = key_event(Some("my-key"), kwargs(vec![]));

    let message =
        render_key_event_message(&event, "Virtual Key Updated").expect("the message renders");

    assert!(message.contains(
        "created_by_user_role: `proxy_admin`\ncreated_by_key_alias: `my-key`\n\n*Arguments passed:*\n"
    ));
}

#[test]
fn the_user_api_key_dict_argument_is_skipped() {
    let event = key_event(
        None,
        kwargs(vec![
            ("user_api_key_dict", Json::Text("secret".to_owned())),
            ("key_alias", Json::Text("my-key".to_owned())),
        ]),
    );

    let message =
        render_key_event_message(&event, "New Virtual Key Created").expect("the message renders");

    assert!(!message.contains("user_api_key_dict"));
    assert!(!message.contains("secret"));
    assert!(message.contains("key_alias: `my-key`\n"));
}

#[rstest]
#[case::bool_value(Json::Bool(false), "false")]
#[case::number_value(Json::Number(Number::from(7_u32)), "7")]
#[case::list_value(Json::List(vec![Json::Bool(true)]), "[true]")]
#[case::object_value(
    Json::Object(IndexMap::from([("a".to_owned(), Json::Number(Number::from(1_u32)))])),
    "{\"a\":1}"
)]
#[case::null_value(Json::Null, "None")]
#[case::text_value(Json::Text("plain".to_owned()), "plain")]
fn renders_argument_values_in_json_text(#[case] value: Json, #[case] expected: &str) {
    let event = key_event(None, kwargs(vec![("argument", value)]));

    let message = render_key_event_message(&event, "Team Created").expect("the message renders");

    assert!(message.contains(&format!("argument: `{expected}`\n")));
}

#[tokio::test]
async fn the_alert_goes_out_high_with_the_given_type() {
    let sender = FakeSender::default();
    let service = alerts(sender.clone());
    let event = key_event(None, kwargs(vec![]));

    service
        .send_virtual_key_event_slack(
            &event,
            AlertType::NewVirtualKeyCreated,
            "New Virtual Key Created",
        )
        .await;

    let sent = sender.alerts.entries();
    let alert = sent.first().expect("one key event alert");
    assert_eq!(alert.level, AlertLevel::High);
    assert_eq!(alert.alert_type, AlertType::NewVirtualKeyCreated);
    assert!(alert.user_info.is_none());
    assert!(alert.metadata.is_empty());
    assert_eq!(
        alert.message,
        render_key_event_message(&event, "New Virtual Key Created").expect("the message renders")
    );
}

#[tokio::test]
async fn a_sender_error_is_logged_and_swallowed() {
    let log = LogCapture::new();
    let service = alerts(FakeSender::failing());
    let event = key_event(None, kwargs(vec![]));

    service
        .send_virtual_key_event_slack(&event, AlertType::VirtualKeyDeleted, "Virtual Key Deleted")
        .await;

    let text = log.text();
    assert!(text.contains("Error sending send_virtual_key_event_slack"));
    assert!(text.contains("the fake sender failed"));
}
