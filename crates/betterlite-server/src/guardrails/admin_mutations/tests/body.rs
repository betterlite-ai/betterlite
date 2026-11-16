fn issue_of(error: BodyError) -> JsonData {
    parse(&serde_json::to_string(&error).expect("the issue serializes"))
}

fn issue_type(issue: &JsonData) -> String {
    text(field(issue, "type")).to_owned()
}

fn issue_loc(issue: &JsonData) -> Vec<JsonData> {
    match field(issue, "loc") {
        JsonData::Array(items) => items.clone(),
        other @ (JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::String(_)
        | JsonData::Object(_)) => panic!("the loc is not a list: {other:?}"),
    }
}

fn loc_field(issue: &JsonData, position: usize) -> String {
    match &issue_loc(issue)[position] {
        JsonData::String(name) => name.clone(),
        other @ (JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::Array(_)
        | JsonData::Object(_)) => panic!("the loc part is not text: {other:?}"),
    }
}

fn issue_message(issue: &JsonData) -> String {
    text(field(issue, "msg")).to_owned()
}

fn create_request(text: &str) -> WriteRequest {
    decode_write_request(text.as_bytes()).expect("the create body decodes")
}

fn create_error(text: &str) -> JsonData {
    let error = decode_write_request(text.as_bytes())
        .err()
        .expect("the create body is rejected");
    issue_of(error)
}

fn create_error_bytes(bytes: &[u8]) -> JsonData {
    let error = decode_write_request(bytes)
        .err()
        .expect("the create body is rejected");
    issue_of(error)
}

#[test]
fn body_create_decodes_the_guardrail() {
    let request = create_request(CREATE_JSON);
    assert_eq!(request.guardrail.guardrail_name, "my-guard");
    assert_eq!(request.guardrail.litellm_params.guardrail, "bedrock");
    assert!(request.guardrail.guardrail_info.is_none());
    assert!(request.guardrail_info.is_none());
}

#[test]
fn body_create_keeps_a_sent_null_info() {
    let body = r#"{"guardrail":{"guardrail_name":"g","litellm_params":{"guardrail":"bedrock","mode":"pre_call"},"guardrail_info":null}}"#;
    let request = create_request(body);
    assert!(request.guardrail.guardrail_info.is_none());
    let sent = request.guardrail_info.expect("a sent null is kept");
    assert_eq!(sent.get(), "null");
}

#[test]
fn body_create_decodes_the_info_object() {
    let request = create_request(CREATE_WITH_INFO);
    let info = request
        .guardrail
        .guardrail_info
        .expect("the info object decodes");
    assert!(info.0.contains_key("description"));
    assert!(request.guardrail_info.is_some());
}

#[test]
fn body_create_ignores_unknown_keys() {
    let body = r#"{"guardrail":{"guardrail_name":"g","litellm_params":{"guardrail":"bedrock","mode":"pre_call"},"extra":"ignored"},"another":1}"#;
    let request = create_request(body);
    assert_eq!(request.guardrail.guardrail_name, "g");
}

#[test]
fn body_create_missing_wrapper_is_reported() {
    let issue = create_error("{}");
    assert_eq!(issue_type(&issue), "missing");
    assert_eq!(loc_field(&issue, 0), "body");
    assert_eq!(loc_field(&issue, 1), "guardrail");
    assert_eq!(issue_loc(&issue).len(), 2);
    assert_eq!(issue_message(&issue), "Field required");
}

#[test]
fn body_create_missing_name_is_reported() {
    let issue = create_error(
        r#"{"guardrail":{"litellm_params":{"guardrail":"bedrock","mode":"pre_call"}}}"#,
    );
    assert_eq!(issue_type(&issue), "missing");
    assert_eq!(loc_field(&issue, 2), "guardrail_name");
    assert_eq!(issue_message(&issue), "Field required");
    let input = field(&issue, "input");
    assert!(matches!(
        field(input, "litellm_params"),
        JsonData::Object(_)
    ));
}

#[test]
fn body_create_missing_params_is_reported() {
    let issue = create_error(r#"{"guardrail":{"guardrail_name":"g"}}"#);
    assert_eq!(issue_type(&issue), "missing");
    assert_eq!(loc_field(&issue, 2), "litellm_params");
    assert_eq!(issue_message(&issue), "Field required");
}

#[test]
fn body_create_null_params_is_reported() {
    let issue = create_error(r#"{"guardrail":{"guardrail_name":"g","litellm_params":null}}"#);
    assert_eq!(issue_type(&issue), "value_error");
    assert_eq!(loc_field(&issue, 2), "litellm_params");
    assert_eq!(issue_loc(&issue).len(), 3);
}

#[test]
fn body_create_empty_params_names_the_field() {
    let issue = create_error(r#"{"guardrail":{"guardrail_name":"g","litellm_params":{}}}"#);
    assert_eq!(issue_type(&issue), "missing");
    assert_eq!(loc_field(&issue, 3), "guardrail");
    assert_eq!(issue_message(&issue), "Field required");
    assert_eq!(field(&issue, "input"), &JsonData::Object(BTreeMap::new()));
}

#[test]
fn body_create_truncated_json_is_reported() {
    let issue = create_error(r#"{"guardrail":{"guardrail_name":"g""#);
    assert_eq!(issue_type(&issue), "json_invalid");
    assert_eq!(loc_field(&issue, 0), "body");
    assert_eq!(issue_message(&issue), "JSON decode error");
    assert_eq!(field(&issue, "input"), &JsonData::Object(BTreeMap::new()));
}

#[test]
fn body_create_trailing_data_is_reported() {
    let issue = create_error(r#"{"guardrail":{}} {}"#);
    assert_eq!(issue_type(&issue), "json_invalid");
    assert_eq!(issue_message(&issue), "JSON decode error");
}

#[test]
fn body_create_bad_utf8_is_reported() {
    let issue = create_error_bytes(&[0xff, 0xfe, 0x00]);
    assert_eq!(issue_type(&issue), "json_invalid");
    assert_eq!(issue_loc(&issue).len(), 1);
}

#[test]
fn body_create_non_object_info_is_reported() {
    let issue = create_error(
        r#"{"guardrail":{"guardrail_name":"g","litellm_params":{"guardrail":"bedrock","mode":"pre_call"},"guardrail_info":5}}"#,
    );
    assert_eq!(issue_type(&issue), "value_error");
    assert_eq!(issue_loc(&issue).len(), 3);
    assert_eq!(loc_field(&issue, 2), "guardrail_info");
    assert!(matches!(field(&issue, "input"), JsonData::Number(_)));
}

#[test]
fn body_create_wrong_type_name_is_reported() {
    let issue = create_error(
        r#"{"guardrail":{"guardrail_name":5,"litellm_params":{"guardrail":"bedrock","mode":"pre_call"}}}"#,
    );
    assert_eq!(issue_type(&issue), "value_error");
    assert_eq!(issue_loc(&issue).len(), 3);
    assert_eq!(loc_field(&issue, 2), "guardrail_name");
    assert!(matches!(field(&issue, "input"), JsonData::Number(_)));
}

#[test]
fn body_create_non_object_body_is_reported() {
    let issue = create_error("[]");
    assert_eq!(issue_type(&issue), "value_error");
    assert_eq!(loc_field(&issue, 0), "body");
    assert_eq!(field(&issue, "input"), &JsonData::Array(Vec::new()));
}

#[test]
fn body_patch_decodes_an_empty_request() {
    let request = decode_patch_body(b"{}").expect("the patch body decodes");
    assert!(request.guardrail_name.is_none());
    assert!(request.litellm_params.is_none());
    assert!(request.guardrail_info.is_none());
}

#[test]
fn body_patch_keeps_an_empty_info_object() {
    let request = decode_patch_body(br#"{"guardrail_info":{}}"#).expect("the patch body decodes");
    let info = request.guardrail_info.expect("an empty object is sent");
    assert!(info.0.is_empty());
}

#[test]
fn body_patch_null_name_and_params_are_not_sent() {
    let request = decode_patch_body(br#"{"guardrail_name":null,"litellm_params":null}"#)
        .expect("the patch body decodes");
    assert!(request.guardrail_name.is_none());
    assert!(request.litellm_params.is_none());
}

#[test]
fn body_patch_bad_params_is_reported() {
    let issue = issue_of(
        decode_patch_body(br#"{"litellm_params":5}"#)
            .err()
            .expect("the patch body is rejected"),
    );
    assert_eq!(issue_type(&issue), "value_error");
    assert_eq!(issue_loc(&issue).len(), 2);
    assert_eq!(loc_field(&issue, 1), "litellm_params");
    assert!(matches!(field(&issue, "input"), JsonData::Number(_)));
}

#[test]
fn body_patch_truncated_json_is_reported() {
    let issue = issue_of(
        decode_patch_body(b"{")
            .err()
            .expect("the patch body is rejected"),
    );
    assert_eq!(issue_type(&issue), "json_invalid");
    assert_eq!(issue_message(&issue), "JSON decode error");
}

#[test]
fn body_patch_bad_utf8_is_reported() {
    let issue = issue_of(
        decode_patch_body(&[0x22, 0xff, 0x22])
            .err()
            .expect("the patch body is rejected"),
    );
    assert_eq!(issue_type(&issue), "json_invalid");
}

#[test]
fn body_patch_trailing_data_is_reported() {
    let issue = issue_of(
        decode_patch_body(b"{} {}")
            .err()
            .expect("the patch body is rejected"),
    );
    assert_eq!(issue_type(&issue), "json_invalid");
}

fn replay_corpus(dir_name: &str) -> usize {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fuzz/corpus")
        .join(dir_name);
    let mut count = 0_usize;
    for entry in fs::read_dir(&root).unwrap() {
        let body = fs::read(entry.unwrap().path()).unwrap();
        drop(decode_write_request(&body));
        drop(decode_patch_body(&body));
        count = count.saturating_add(1);
    }
    count
}

#[test]
fn replays_the_guardrail_write_corpus() {
    assert!(replay_corpus("guardrail_write_body") > 0);
}

#[tokio::test]
async fn body_validation_answers_422_in_the_fastapi_shape() {
    let harness = harness();
    let (status, body) = send(
        harness.router,
        request("POST", "/guardrails", Some(admin()), Some("{}")),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let issue = first_issue(&body);
    assert_eq!(keys(&issue), vec!["input", "loc", "msg", "type"]);
    assert_eq!(issue_type(&issue), "missing");
    assert_eq!(issue_message(&issue), "Field required");
}

#[tokio::test]
async fn body_patch_validation_answers_422_in_the_fastapi_shape() {
    let harness = harness();
    let (status, body) = send(
        harness.router,
        request("PATCH", "/guardrails/g1", Some(admin()), Some("nope")),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let issue = first_issue(&body);
    assert_eq!(issue_type(&issue), "json_invalid");
    assert_eq!(loc_field(&issue, 0), "body");
}

#[tokio::test]
async fn auth_create_answers_401_before_the_body() {
    let harness = harness();
    let (missing, _body) = send(
        harness.router.clone(),
        request("POST", "/guardrails", None, Some("{}")),
    )
    .await;
    assert_eq!(missing, StatusCode::UNAUTHORIZED);
    let (valid, body) = send(
        harness.router,
        request("POST", "/guardrails", None, Some(CREATE_JSON)),
    )
    .await;
    assert_eq!(valid, StatusCode::UNAUTHORIZED);
    assert!(body.contains("auth_error"));
}

#[tokio::test]
async fn auth_all_six_routes_answer_401() {
    let harness = harness();
    let (create, _body) = send(
        harness.router.clone(),
        request("POST", "/guardrails", None, Some(CREATE_JSON)),
    )
    .await;
    assert_eq!(create, StatusCode::UNAUTHORIZED);
    let (replace, _body) = send(
        harness.router.clone(),
        request("PUT", "/guardrails/g1", None, Some(CREATE_JSON)),
    )
    .await;
    assert_eq!(replace, StatusCode::UNAUTHORIZED);
    let (patch, _body) = send(
        harness.router.clone(),
        request("PATCH", "/guardrails/g1", None, Some("{}")),
    )
    .await;
    assert_eq!(patch, StatusCode::UNAUTHORIZED);
    let (delete, _body) = send(
        harness.router.clone(),
        request("DELETE", "/guardrails/g1", None, None),
    )
    .await;
    assert_eq!(delete, StatusCode::UNAUTHORIZED);
    let (approve, _body) = send(
        harness.router.clone(),
        request("POST", "/guardrails/submissions/g1/approve", None, None),
    )
    .await;
    assert_eq!(approve, StatusCode::UNAUTHORIZED);
    let (reject, _body) = send(
        harness.router,
        request("POST", "/guardrails/submissions/g1/reject", None, None),
    )
    .await;
    assert_eq!(reject, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn auth_bad_body_is_a_422_for_any_caller() {
    let harness = harness();
    let (viewer, _body) = send(
        harness.router.clone(),
        request("POST", "/guardrails", Some(viewer()), Some("{}")),
    )
    .await;
    assert_eq!(viewer, StatusCode::UNPROCESSABLE_ENTITY);
    let (role_admin, _body) = send(
        harness.router,
        request("POST", "/guardrails", Some(role_admin()), Some("{}")),
    )
    .await;
    assert_eq!(role_admin, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn auth_viewer_is_forbidden_on_the_guardrail_routes() {
    let harness = harness();
    let routes = [
        ("POST", "/guardrails", Some(CREATE_JSON)),
        ("PUT", "/guardrails/g1", Some(CREATE_JSON)),
        ("PATCH", "/guardrails/g1", Some("{}")),
        ("DELETE", "/guardrails/g1", None),
    ];
    for (method, uri, payload) in routes {
        let (status, body) = send(
            harness.router.clone(),
            request(method, uri, Some(viewer()), payload),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}");
        assert_eq!(
            detail(&body),
            "Admin access required to manage guardrails",
            "{method} {uri}"
        );
    }
}

#[tokio::test]
async fn auth_viewer_is_forbidden_on_the_review_routes() {
    let harness = harness();
    for uri in [
        "/guardrails/submissions/g1/approve",
        "/guardrails/submissions/g1/reject",
    ] {
        let (status, body) = send(
            harness.router.clone(),
            request("POST", uri, Some(viewer()), None),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{uri}");
        assert_eq!(detail(&body), "Admin access required", "{uri}");
    }
}

#[tokio::test]
async fn auth_no_database_is_a_500_on_every_route() {
    let router = bare_router();
    let routes = [
        ("POST", "/guardrails", Some(CREATE_JSON)),
        ("PUT", "/guardrails/g1", Some(CREATE_JSON)),
        ("PATCH", "/guardrails/g1", Some("{}")),
        ("DELETE", "/guardrails/g1", None),
        ("POST", "/guardrails/submissions/g1/approve", None),
        ("POST", "/guardrails/submissions/g1/reject", None),
    ];
    for (method, uri, payload) in routes {
        let (status, body) =
            send(router.clone(), request(method, uri, Some(admin()), payload)).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{method} {uri}");
        assert_eq!(
            detail(&body),
            "Prisma client not initialized",
            "{method} {uri}"
        );
    }
}
