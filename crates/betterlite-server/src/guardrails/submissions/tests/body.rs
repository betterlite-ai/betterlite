#[tokio::test]
async fn body_ignores_unknown_keys() {
    let harness = harness();
    let body = format!(
        r#"{{"guardrail_name":"guard","litellm_params":{REGISTER_PARAMS},"team_id":"team-a","unknown":{{"a":1}}}}"#
    );
    let answer = send(
        &harness.router,
        post("/guardrails/register", Some(admin()), &body),
    )
    .await;
    assert_eq!(answer.0, StatusCode::OK, "{}", answer.1);
    assert_eq!(harness.rows.created().len(), 1);
}

#[tokio::test]
async fn body_without_guardrail_name_is_422() {
    let harness = harness();
    let body = format!(r#"{{"litellm_params":{REGISTER_PARAMS}}}"#);
    let answer = send(
        &harness.router,
        post("/guardrails/register", Some(admin()), &body),
    )
    .await;
    assert_eq!(answer.0, StatusCode::UNPROCESSABLE_ENTITY);
    let issue = first_issue(&answer.1);
    assert_eq!(text(field(&issue, "type")), "missing");
    assert_eq!(loc_of(&issue), vec!["body", "guardrail_name"]);
    assert_eq!(text(field(&issue, "msg")), "Field required");
    let input = field(&issue, "input");
    assert_eq!(field(input, "litellm_params"), &parse(REGISTER_PARAMS));
    assert!(harness.rows.created().is_empty());
}

#[tokio::test]
async fn body_without_litellm_params_is_422() {
    let harness = harness();
    let body = r#"{"guardrail_name":"guard"}"#;
    let answer = send(
        &harness.router,
        post("/guardrails/register", Some(admin()), body),
    )
    .await;
    assert_eq!(answer.0, StatusCode::UNPROCESSABLE_ENTITY);
    let issue = first_issue(&answer.1);
    assert_eq!(text(field(&issue, "type")), "missing");
    assert_eq!(loc_of(&issue), vec!["body", "litellm_params"]);
    assert_eq!(
        text(field(field(&issue, "input"), "guardrail_name")),
        "guard"
    );
    assert!(harness.rows.created().is_empty());
}

#[tokio::test]
async fn body_with_a_non_object_litellm_params_is_422() {
    let harness = harness();
    let body = r#"{"guardrail_name":"guard","litellm_params":"not-a-map"}"#;
    let answer = send(
        &harness.router,
        post("/guardrails/register", Some(admin()), body),
    )
    .await;
    assert_eq!(answer.0, StatusCode::UNPROCESSABLE_ENTITY);
    let issue = first_issue(&answer.1);
    assert_eq!(text(field(&issue, "type")), "value_error");
    assert_eq!(loc_of(&issue), vec!["body", "litellm_params"]);
    assert_eq!(text(field(&issue, "input")), "not-a-map");
}

#[tokio::test]
async fn body_with_malformed_json_is_422() {
    let harness = harness();
    let answer = send(
        &harness.router,
        post("/guardrails/register", Some(admin()), "not json"),
    )
    .await;
    assert_eq!(answer.0, StatusCode::UNPROCESSABLE_ENTITY);
    let issue = first_issue(&answer.1);
    assert_eq!(text(field(&issue, "type")), "json_invalid");
    assert_eq!(loc_of(&issue), vec!["body"]);
    assert_eq!(text(field(&issue, "msg")), "JSON decode error");
}

#[tokio::test]
async fn body_with_truncated_json_is_422() {
    let harness = harness();
    let answer = send(
        &harness.router,
        post(
            "/guardrails/register",
            Some(admin()),
            r#"{"guardrail_name":"#,
        ),
    )
    .await;
    assert_eq!(answer.0, StatusCode::UNPROCESSABLE_ENTITY);
    let issue = first_issue(&answer.1);
    assert_eq!(text(field(&issue, "type")), "json_invalid");
    assert_eq!(loc_of(&issue)[0], "body");
    assert_eq!(text(field(&issue, "msg")), "JSON decode error");
}

#[tokio::test]
async fn body_with_trailing_data_is_422() {
    let harness = harness();
    let body = r#"{"guardrail_name":"guard","litellm_params":{}} {}"#;
    let answer = send(
        &harness.router,
        post("/guardrails/register", Some(admin()), body),
    )
    .await;
    assert_eq!(answer.0, StatusCode::UNPROCESSABLE_ENTITY);
    let issue = first_issue(&answer.1);
    assert_eq!(text(field(&issue, "type")), "json_invalid");
    assert_eq!(loc_of(&issue), vec!["body"]);
}
