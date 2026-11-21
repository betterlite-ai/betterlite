const GUARDRAIL_TYPE: &str = "Only guardrails with litellm_params.guardrail='generic_guardrail_api' are accepted for registration";
const API_BASE_REQUIRED: &str = "litellm_params.api_base is required for generic_guardrail_api";
const API_BASE_SCHEME: &str = "litellm_params.api_base must use http or https scheme";
const API_BASE_HOST: &str = "litellm_params.api_base must contain a valid hostname";
const MODE_REQUIRED: &str = "litellm_params.mode is required (e.g. pre_call, post_call)";

fn params_with(api_base: &str) -> String {
    format!(r#"{{"guardrail":"generic_guardrail_api","mode":"pre_call","api_base":{api_base}}}"#)
}

#[rstest]
#[case::wrong_type(
    r#"{"guardrail":"bedrock","mode":"pre_call","api_base":"https://guard.example.com/check"}"#
)]
#[case::missing_type(r#"{"mode":"pre_call","api_base":"https://guard.example.com/check"}"#)]
#[tokio::test]
async fn register_requires_the_generic_guardrail_api_type(#[case] stored: &str) {
    let harness = harness();
    let answer = register(
        &harness,
        admin(),
        &register_request("guard", stored, Some("team-a")),
    )
    .await;
    assert_error(&answer, StatusCode::BAD_REQUEST, GUARDRAIL_TYPE);
    assert!(harness.rows.created().is_empty());
}

#[rstest]
#[case::missing(r#"{"guardrail":"generic_guardrail_api","mode":"pre_call"}"#)]
#[case::null(r#"{"guardrail":"generic_guardrail_api","mode":"pre_call","api_base":null}"#)]
#[case::empty(r#"{"guardrail":"generic_guardrail_api","mode":"pre_call","api_base":""}"#)]
#[tokio::test]
async fn register_requires_an_api_base(#[case] stored: &str) {
    let harness = harness();
    let answer = register(
        &harness,
        admin(),
        &register_request("guard", stored, Some("team-a")),
    )
    .await;
    assert_error(&answer, StatusCode::BAD_REQUEST, API_BASE_REQUIRED);
}

#[rstest]
#[case::number("42")]
#[case::float("42.5")]
#[case::boolean("true")]
#[case::array(r#"["https://guard.example.com"]"#)]
#[case::object(r#"{"url":"https://guard.example.com"}"#)]
#[tokio::test]
async fn register_answers_a_non_text_api_base_as_a_scheme_error(#[case] value: &str) {
    let harness = harness();
    let answer = register(
        &harness,
        admin(),
        &register_request("guard", &params_with(value), Some("team-a")),
    )
    .await;
    assert_error(&answer, StatusCode::BAD_REQUEST, API_BASE_SCHEME);
}

#[rstest]
#[case::plain("plain", "\"http://guard.example.com/check\"", None)]
#[case::secure("secure", "\"https://guard.example.com/check\"", None)]
#[case::ftp("refused", "\"ftp://guard.example.com/check\"", Some(API_BASE_SCHEME))]
#[case::host_only("refused", "\"guard.example.com/check\"", Some(API_BASE_SCHEME))]
#[case::authority("refused", "\"//guard.example.com/check\"", Some(API_BASE_SCHEME))]
#[tokio::test]
async fn register_accepts_http_and_https_and_refuses_other_schemes(
    #[case] name: &str,
    #[case] base: &str,
    #[case] refused: Option<&str>,
) {
    let harness = harness();
    let answer = register(
        &harness,
        admin(),
        &register_request(name, &params_with(base), Some("team-a")),
    )
    .await;
    match refused {
        Some(message) => assert_error(&answer, StatusCode::BAD_REQUEST, message),
        None => assert_eq!(answer.0, StatusCode::OK, "{}", answer.1),
    }
}

#[rstest]
#[case::blank_host("\"http://\"")]
#[case::empty_host("\"https:///check\"")]
#[tokio::test]
async fn register_requires_a_hostname(#[case] base: &str) {
    let harness = harness();
    let answer = register(
        &harness,
        admin(),
        &register_request("guard", &params_with(base), Some("team-a")),
    )
    .await;
    assert_error(&answer, StatusCode::BAD_REQUEST, API_BASE_HOST);
}

#[rstest]
#[case::missing(
    r#"{"guardrail":"generic_guardrail_api","api_base":"https://guard.example.com/check"}"#
)]
#[case::null(
    r#"{"guardrail":"generic_guardrail_api","api_base":"https://guard.example.com/check","mode":null}"#
)]
#[tokio::test]
async fn register_requires_a_mode(#[case] stored: &str) {
    let harness = harness();
    let answer = register(
        &harness,
        admin(),
        &register_request("guard", stored, Some("team-a")),
    )
    .await;
    assert_error(&answer, StatusCode::BAD_REQUEST, MODE_REQUIRED);
}

#[tokio::test]
async fn register_runs_the_checks_in_the_order_litellm_does() {
    let harness = harness();
    let no_team = register(
        &harness,
        user_key(None, "alice"),
        &register_request("guard", r#"{"guardrail":"bedrock"}"#, None),
    )
    .await;
    assert_error(&no_team, StatusCode::BAD_REQUEST, TEAM_REQUIRED);
    harness.teams.allow("alice", &["team-a"]);
    let outsider = register(
        &harness,
        user_key(Some("team-a"), "alice"),
        &register_request(
            "guard",
            r#"{"guardrail":"bedrock","api_base":"ftp://guard.example.com"}"#,
            Some("team-b"),
        ),
    )
    .await;
    assert_error(
        &outsider,
        StatusCode::FORBIDDEN,
        "You are not a member of team 'team-b'",
    );
    let scheme_first = register(
        &harness,
        admin(),
        &register_request(
            "guard",
            r#"{"guardrail":"generic_guardrail_api","api_base":"ftp://guard.example.com"}"#,
            Some("team-a"),
        ),
    )
    .await;
    assert_error(&scheme_first, StatusCode::BAD_REQUEST, API_BASE_SCHEME);
    harness
        .rows
        .insert(row("g1", "guard", Some("active"), Some("team-b")));
    let mode_first = register(
        &harness,
        admin(),
        &register_request(
            "guard",
            r#"{"guardrail":"generic_guardrail_api","api_base":"https://guard.example.com/check"}"#,
            Some("team-a"),
        ),
    )
    .await;
    assert_error(&mode_first, StatusCode::BAD_REQUEST, MODE_REQUIRED);
}
