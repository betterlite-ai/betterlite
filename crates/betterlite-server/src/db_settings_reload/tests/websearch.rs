use std::sync::atomic::Ordering;

use betterlite_config::sections::settings::JsonValue;
use betterlite_config::sections::websearch_request_hooks::Settings as WebsearchParams;
use rstest::rstest;

use super::support::{ControlLog, FakeCallbacks, FakeRows, capture_logs, fault, json, reloader};

fn rows_with(params: &str) -> FakeRows {
    FakeRows::litellm(json(&format!(
        "{{\"websearch_interception_params\":{params}}}"
    )))
}

async fn run(params: &str) -> (Vec<Option<WebsearchParams>>, String) {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let callbacks = FakeCallbacks::default();
    let (logs, _guard) = capture_logs();
    reloader
        .reload_websearch(&rows_with(params), &callbacks)
        .await;
    (callbacks.applied(), logs.contents())
}

fn enabled_params() -> WebsearchParams {
    WebsearchParams {
        enabled_providers: vec!["bedrock".to_owned()],
        search_tool_name: Some("my-search".to_owned()),
        max_agentic_loops: None,
    }
}

#[tokio::test]
async fn an_absent_row_applies_nothing() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let callbacks = FakeCallbacks::default();
    let (logs, _guard) = capture_logs();
    reloader
        .reload_websearch(&FakeRows::default(), &callbacks)
        .await;
    assert!(callbacks.applied().is_empty());
    assert_eq!(reloader.last_websearch, None);
    assert_eq!(logs.contents(), "");
}

#[tokio::test]
async fn an_absent_params_key_applies_nothing() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let callbacks = FakeCallbacks::default();
    let rows = FakeRows::litellm(json("{\"other\":true}"));
    let (logs, _guard) = capture_logs();
    reloader.reload_websearch(&rows, &callbacks).await;
    assert!(callbacks.applied().is_empty());
    assert_eq!(logs.contents(), "");
}

#[tokio::test]
async fn a_params_value_that_is_not_a_mapping_applies_nothing() {
    let (applied, logs) = run("\"yes\"").await;
    assert!(applied.is_empty());
    assert_eq!(logs, "");
}

#[tokio::test]
async fn a_json_string_row_is_decoded_and_applied() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let callbacks = FakeCallbacks::default();
    let row = FakeRows::litellm(JsonValue::String(
        "{\"websearch_interception_params\":{\"enabled\":true,\"search_tool_name\":\"my-search\"}}"
            .to_owned(),
    ));
    let (logs, _guard) = capture_logs();
    reloader.reload_websearch(&row, &callbacks).await;
    assert_eq!(callbacks.applied(), vec![Some(enabled_params())]);
    assert!(
        logs.contents()
            .contains("Web search interception reinitialized from DB")
    );
}

#[rstest]
#[case::array(json("[]"))]
#[case::number(json("5"))]
#[case::bool(json("true"))]
#[case::double_encoded_null(JsonValue::String("null".to_owned()))]
#[tokio::test]
async fn a_litellm_settings_that_is_not_an_object_is_logged(#[case] stored: JsonValue) {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let callbacks = FakeCallbacks::default();
    let row = FakeRows::litellm(stored);
    let (logs, _guard) = capture_logs();
    reloader.reload_websearch(&row, &callbacks).await;
    assert!(callbacks.applied().is_empty());
    assert_eq!(reloader.last_websearch, None);
    assert!(
        logs.contents()
            .contains("Error initializing web search interception settings from DB: litellm_settings is not a JSON object")
    );
}

#[tokio::test]
async fn a_read_failure_is_logged_and_applies_nothing() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let callbacks = FakeCallbacks::default();
    let rows = FakeRows::failing(fault("the database is down"));
    let (logs, _guard) = capture_logs();
    reloader.reload_websearch(&rows, &callbacks).await;
    assert!(callbacks.applied().is_empty());
    assert_eq!(reloader.last_websearch, None);
    assert!(
        logs.contents().contains(
            "Error initializing web search interception settings from DB: could not read the settings row: the database is down"
        )
    );
}

#[tokio::test]
async fn a_config_without_enabled_leaves_the_callback_to_the_configured_callbacks() {
    let (applied, logs) = run("{\"search_tool_name\":\"my-search\"}").await;
    assert!(applied.is_empty());
    assert!(logs.contains(
        "Web search interception: stored settings carry no 'enabled' flag and none were applied before, so litellm_settings.callbacks keeps ownership of the callback."
    ));
}

#[tokio::test]
async fn an_unchanged_config_without_enabled_and_a_registered_callback_does_nothing() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    reloader.last_websearch = Some(json("{\"search_tool_name\":\"my-search\"}"));
    let callbacks = FakeCallbacks::default();
    callbacks.registered.store(true, Ordering::SeqCst);
    let (logs, _guard) = capture_logs();
    reloader
        .reload_websearch(
            &rows_with("{\"search_tool_name\":\"my-search\"}"),
            &callbacks,
        )
        .await;
    assert!(callbacks.applied().is_empty());
    assert_eq!(logs.contents(), "");
}

#[tokio::test]
async fn a_config_without_enabled_reapplies_when_the_callback_is_missing() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    reloader.last_websearch = Some(json("{\"search_tool_name\":\"my-search\"}"));
    let callbacks = FakeCallbacks::default();
    let (logs, _guard) = capture_logs();
    reloader
        .reload_websearch(
            &rows_with("{\"search_tool_name\":\"my-search\"}"),
            &callbacks,
        )
        .await;
    assert_eq!(callbacks.applied(), vec![Some(enabled_params())]);
    assert!(
        logs.contents()
            .contains("Web search interception reinitialized from DB")
    );
}

#[tokio::test]
async fn a_repeated_config_does_nothing() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let callbacks = FakeCallbacks::default();
    let rows = rows_with("{\"enabled\":true,\"search_tool_name\":\"my-search\"}");
    let (logs, _guard) = capture_logs();
    reloader.reload_websearch(&rows, &callbacks).await;
    reloader.reload_websearch(&rows, &callbacks).await;
    assert_eq!(callbacks.applied().len(), 1);
    assert_eq!(
        logs.count("Web search interception reinitialized from DB"),
        1
    );
}

#[tokio::test]
async fn a_changed_config_reapplies() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let callbacks = FakeCallbacks::default();
    let (_logs, _guard) = capture_logs();
    reloader
        .reload_websearch(
            &rows_with("{\"enabled\":true,\"search_tool_name\":\"a\"}"),
            &callbacks,
        )
        .await;
    reloader
        .reload_websearch(
            &rows_with("{\"enabled\":true,\"search_tool_name\":\"b\"}"),
            &callbacks,
        )
        .await;
    let applied = callbacks.applied();
    assert_eq!(applied.len(), 2);
    assert_eq!(
        applied[1].as_ref().unwrap().search_tool_name.as_deref(),
        Some("b")
    );
}

#[tokio::test]
async fn a_callback_state_that_disagrees_reapplies_the_same_config() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let callbacks = FakeCallbacks::default();
    let rows = rows_with("{\"enabled\":true,\"search_tool_name\":\"my-search\"}");
    let (_logs, _guard) = capture_logs();
    reloader.reload_websearch(&rows, &callbacks).await;
    callbacks.registered.store(false, Ordering::SeqCst);
    reloader.reload_websearch(&rows, &callbacks).await;
    assert_eq!(callbacks.applied().len(), 2);
}

#[rstest]
#[case::null("null", false)]
#[case::false_value("false", false)]
#[case::true_value("true", true)]
#[case::lower_true("\"true\"", true)]
#[case::upper_true("\"TRUE\"", true)]
#[case::not_true_word("\"yes\"", false)]
#[case::one("1", true)]
#[case::zero("0", false)]
#[case::empty_list("[]", false)]
#[case::filled_list("[1]", true)]
#[case::empty_object("{}", false)]
#[case::filled_object("{\"a\":1}", true)]
#[tokio::test]
async fn the_enabled_flag_is_coerced(#[case] stored: &str, #[case] expected: bool) {
    let (applied, logs) = run(&format!(
        "{{\"enabled\":{stored},\"search_tool_name\":\"my-search\"}}"
    ))
    .await;
    if expected {
        assert_eq!(applied, vec![Some(enabled_params())]);
        assert!(logs.contains("Web search interception reinitialized from DB"));
    } else {
        assert_eq!(applied, vec![None]);
        assert!(logs.contains("Web search interception disabled"));
    }
}

#[rstest]
#[case::absent("", vec!["bedrock"])]
#[case::empty_list("\"enabled_providers\":[],", vec!["bedrock"])]
#[case::null("\"enabled_providers\":null,", vec!["bedrock"])]
#[case::bare_string("\"enabled_providers\":\"tavily\",", vec!["bedrock"])]
#[case::one("\"enabled_providers\":[\"tavily\"],", vec!["tavily"])]
#[case::two(
    "\"enabled_providers\":[\"tavily\",\"bedrock\"],",
    vec!["tavily", "bedrock"]
)]
#[tokio::test]
async fn enabled_providers_is_dropped_unless_it_is_a_filled_list(
    #[case] stored: &str,
    #[case] expected: Vec<&str>,
) {
    let (applied, _logs) = run(&format!("{{{stored}\"enabled\":true}}")).await;
    let params = applied[0].as_ref().unwrap();
    let names: Vec<String> = expected.into_iter().map(str::to_owned).collect();
    assert_eq!(params.enabled_providers, names);
}

#[tokio::test]
async fn a_provider_that_is_not_a_name_is_an_error() {
    let (applied, logs) = run("{\"enabled\":true,\"enabled_providers\":[\"ok\",1]}").await;
    assert!(applied.is_empty());
    assert!(logs.contains(
        "Error initializing web search interception settings from DB: websearch_interception_params.enabled_providers must be a list of provider names"
    ));
}

#[tokio::test]
async fn a_search_tool_name_that_is_not_a_string_is_an_error() {
    let (applied, logs) = run("{\"enabled\":true,\"search_tool_name\":5}").await;
    assert!(applied.is_empty());
    assert!(logs.contains(
        "Error initializing web search interception settings from DB: websearch_interception_params.search_tool_name must be a string"
    ));
}

#[tokio::test]
async fn a_null_search_tool_name_stays_none() {
    let (applied, _logs) = run("{\"enabled\":true,\"search_tool_name\":null}").await;
    let params = applied[0].as_ref().unwrap();
    assert_eq!(params.search_tool_name, None);
    assert_eq!(params.max_agentic_loops, None);
}

#[rstest]
#[case::integer("5", Some(5))]
#[case::whole_float("5.0", Some(5))]
#[case::digit_string("\"4\"", Some(4))]
#[case::null("null", None)]
#[tokio::test]
async fn the_agentic_loop_ceiling_is_validated(
    #[case] stored: &str,
    #[case] expected: Option<u32>,
) {
    let (applied, _logs) = run(&format!(
        "{{\"enabled\":true,\"max_agentic_loops\":{stored}}}"
    ))
    .await;
    let params = applied[0].as_ref().unwrap();
    assert_eq!(params.max_agentic_loops, expected);
}

#[rstest]
#[case::zero("0")]
#[case::fraction("2.5")]
#[case::bool_value("true")]
#[case::list("[1]")]
#[tokio::test]
async fn an_invalid_agentic_loop_ceiling_is_an_error(#[case] stored: &str) {
    let (applied, logs) = run(&format!(
        "{{\"enabled\":true,\"max_agentic_loops\":{stored}}}"
    ))
    .await;
    assert!(applied.is_empty());
    assert!(logs.contains(
        "Error initializing web search interception settings from DB: websearch_interception_params.max_agentic_loops"
    ));
}

#[tokio::test]
async fn an_invalid_param_does_not_take_the_callback_away() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let callbacks = FakeCallbacks::default();
    callbacks.registered.store(true, Ordering::SeqCst);
    let (logs, _guard) = capture_logs();
    reloader
        .reload_websearch(
            &rows_with("{\"enabled\":true,\"max_agentic_loops\":0}"),
            &callbacks,
        )
        .await;
    assert!(logs.contents().contains("must be at least 1"));
    assert!(callbacks.applied().is_empty());
    assert!(callbacks.registered.load(Ordering::SeqCst));
    assert_eq!(reloader.last_websearch, None);
}

#[tokio::test]
async fn the_unknown_keys_of_a_stored_config_are_ignored() {
    let (applied, _logs) = run("{\"enabled\":true,\"future_key\":42}").await;
    assert_eq!(
        applied[0],
        Some(WebsearchParams {
            enabled_providers: vec!["bedrock".to_owned()],
            search_tool_name: None,
            max_agentic_loops: None,
        })
    );
}

#[tokio::test]
async fn a_stored_config_is_remembered_as_the_raw_value() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let callbacks = FakeCallbacks::default();
    let rows = rows_with("{\"enabled\":true,\"search_tool_name\":\"my-search\"}");
    let (_logs, _guard) = capture_logs();
    reloader.reload_websearch(&rows, &callbacks).await;
    assert_eq!(
        reloader.last_websearch,
        Some(json(
            "{\"enabled\":true,\"search_tool_name\":\"my-search\"}"
        ))
    );
}
