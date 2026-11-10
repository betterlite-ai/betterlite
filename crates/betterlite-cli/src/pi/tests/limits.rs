#[test]
fn failure_classification_limits_read_model_group_windows() {
    let get = ScriptedGet::new(vec![body(
        200,
        r#"{"data":[{"model_group":"a","max_input_tokens":128000,"max_output_tokens":4096},{"model_group":"b"}]}"#,
    )]);
    let limits = fetch_model_limits(BASE_URL, &api_key(), &get);
    assert_eq!(
        limits.get("a"),
        Some(&ModelLimits {
            context_window: Some(128_000),
            max_tokens: Some(4096),
        })
    );
    assert_eq!(
        limits.get("b"),
        Some(&ModelLimits {
            context_window: None,
            max_tokens: None,
        })
    );
    let request = get.requests().pop().unwrap();
    assert_eq!(request.url, "https://proxy.test/model_group/info");
    assert_eq!(
        request.headers,
        vec![("Authorization".to_owned(), format!("Bearer {API_KEY}"))]
    );
    assert_eq!(request.timeout, Duration::from_secs(10));
}

#[test]
fn failure_classification_limits_strip_trailing_slashes() {
    let get = ScriptedGet::new(vec![body(
        200,
        r#"{"data":[{"model_group":"a","max_input_tokens":1}]}"#,
    )]);
    let limits = fetch_model_limits("https://proxy.test/litellm/", &api_key(), &get);
    assert_eq!(limits.len(), 1);
    assert_eq!(
        get.urls(),
        vec!["https://proxy.test/litellm/model_group/info".to_owned()]
    );
}

#[test]
fn failure_classification_limits_keep_an_explicit_zero_window() {
    let get = ScriptedGet::new(vec![body(
        200,
        r#"{"data":[{"model_group":"a","max_input_tokens":0,"max_output_tokens":0}]}"#,
    )]);
    let limits = fetch_model_limits(BASE_URL, &api_key(), &get);
    assert_eq!(
        limits.get("a"),
        Some(&ModelLimits {
            context_window: Some(0),
            max_tokens: Some(0),
        })
    );
}

#[test]
fn failure_classification_limits_truncate_fractional_windows() {
    let get = ScriptedGet::new(vec![body(
        200,
        r#"{"data":[{"model_group":"a","max_input_tokens":128000.9,"max_output_tokens":-1.9}]}"#,
    )]);
    let limits = fetch_model_limits(BASE_URL, &api_key(), &get);
    assert_eq!(
        limits.get("a"),
        Some(&ModelLimits {
            context_window: Some(128_000),
            max_tokens: Some(-1),
        })
    );
}

#[test]
fn failure_classification_limits_keep_the_last_duplicate_group() {
    let get = ScriptedGet::new(vec![body(
        200,
        r#"{"data":[{"model_group":"a","max_input_tokens":1},{"model_group":"a","max_input_tokens":2}]}"#,
    )]);
    let limits = fetch_model_limits(BASE_URL, &api_key(), &get);
    assert_eq!(limits["a"].context_window, Some(2));
}

#[test]
fn failure_classification_limits_ignore_a_transport_error() {
    let get = ScriptedGet::new(vec![GetAnswer::Transport("offline")]);
    assert!(fetch_model_limits(BASE_URL, &api_key(), &get).is_empty());
}

#[rstest]
#[case::forbidden(403)]
#[case::not_found(404)]
#[case::server_error(500)]
fn failure_classification_limits_ignore_non_200(#[case] status: u16) {
    let get = ScriptedGet::new(vec![body(
        status,
        r#"{"data":[{"model_group":"a","max_input_tokens":100}]}"#,
    )]);
    assert!(fetch_model_limits(BASE_URL, &api_key(), &get).is_empty());
}

#[rstest]
#[case::invalid_json("not json")]
#[case::missing_data("{}")]
#[case::data_not_an_array(r#"{"data":{}}"#)]
#[case::missing_model_group(r#"{"data":[{"max_input_tokens":100}]}"#)]
#[case::string_window(r#"{"data":[{"model_group":"a","max_input_tokens":"100"}]}"#)]
fn failure_classification_limits_ignore_an_unusable_body(#[case] text: &str) {
    let get = ScriptedGet::new(vec![body(200, text)]);
    assert!(fetch_model_limits(BASE_URL, &api_key(), &get).is_empty());
}

#[test]
fn failure_classification_limits_ignore_a_window_outside_the_integer_range() {
    let get = ScriptedGet::new(vec![body(
        200,
        r#"{"data":[{"model_group":"a","max_input_tokens":1e300,"max_output_tokens":4096}]}"#,
    )]);
    let limits = fetch_model_limits(BASE_URL, &api_key(), &get);
    assert_eq!(
        limits.get("a"),
        Some(&ModelLimits {
            context_window: None,
            max_tokens: Some(4096),
        })
    );
}
