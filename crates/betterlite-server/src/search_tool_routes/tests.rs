#![cfg(test)]

mod support;

use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::http::StatusCode;
use betterlite_db::search_tools::convert::isoformat;
use betterlite_models::decisions::Json;
use betterlite_pyhost::json_values::JsonValue;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::connection::SEARCH_PROVIDERS;
use crate::auth::auth_identity::AuthContext;
use support::{
    Fixture, TeamAnswer, build, caller, capture, config_tool, fixed_now, fixture, key_caller,
    log_text, master, moment, names, parse, record, send,
};

const CREATE_BODY: &str = r#"{"search_tool":{"search_tool_name":"fresh","litellm_params":{"search_provider":"tavily","api_key":"sk-plain-secret"},"search_tool_info":{"description":"new"}}}"#;
const UPDATE_BODY: &str = r#"{"search_tool":{"search_tool_name":"renamed","litellm_params":{"search_provider":"brave","api_key":"sk-new-secret-key"}}}"#;
const CONNECT_BODY: &str = r#"{"litellm_params":{"search_provider":"tavily","api_key":"tvly-test-key","api_base":"https://search.example","region":"eu"}}"#;
const MISSING_PROVIDER: &str = "400: search_provider is required in litellm_params";
const TEAM_ONE_ALLOWS_CONFIG: &str = r#"{"team_id":"team-1","object_permission":{"object_permission_id":"perm-team","search_tools":["cfg-a"]}}"#;

#[rstest]
#[case::list("GET", "/search_tools/list", "")]
#[case::create("POST", "/search_tools", CREATE_BODY)]
#[case::update("PUT", "/search_tools/tool-1", UPDATE_BODY)]
#[case::delete("DELETE", "/search_tools/tool-1", "")]
#[case::info("GET", "/search_tools/tool-1", "")]
#[case::test_connection("POST", "/search_tools/test_connection", CONNECT_BODY)]
#[case::providers("GET", "/search_tools/ui/available_providers", "")]
#[tokio::test]
async fn routes_answer_401_without_a_key(
    #[case] method: &str,
    #[case] uri: &str,
    #[case] body: &str,
) {
    let harness = build(fixture());
    let (status, text) = send(&harness, method, uri, body, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(parse(&text)["error"]["code"], "401");
}

#[rstest]
#[case::list("GET", "/search_tools/list", "")]
#[case::create("POST", "/search_tools", CREATE_BODY)]
#[case::update("PUT", "/search_tools/tool-1", UPDATE_BODY)]
#[case::delete("DELETE", "/search_tools/tool-1", "")]
#[case::info("GET", "/search_tools/tool-1", "")]
#[tokio::test]
async fn routes_table_routes_answer_500_without_a_database(
    #[case] method: &str,
    #[case] uri: &str,
    #[case] body: &str,
) {
    let mut setup = fixture();
    setup.database = false;
    let harness = build(setup);
    let (status, text) = send(&harness, method, uri, body, Some(master())).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(parse(&text)["detail"], "Prisma client not initialized");
}

#[rstest]
#[case::empty_body("POST", "/search_tools", "", "missing", r#"["body"]"#)]
#[case::null_body("POST", "/search_tools", "null", "missing", r#"["body"]"#)]
#[case::not_json("POST", "/search_tools", "{nope", "json_invalid", r#"["body"]"#)]
#[case::trailing_bytes(
    "POST",
    "/search_tools",
    r#"{"search_tool":{"search_tool_name":"x","litellm_params":{"search_provider":"tavily"}}} x"#,
    "json_invalid",
    r#"["body"]"#
)]
#[case::no_search_tool("POST", "/search_tools", "{}", "missing", r#"["body","search_tool"]"#)]
#[case::no_name(
    "PUT",
    "/search_tools/tool-1",
    r#"{"search_tool":{"litellm_params":{"search_provider":"tavily"}}}"#,
    "missing",
    r#"["body","search_tool","search_tool_name"]"#
)]
#[case::no_provider(
    "POST",
    "/search_tools",
    r#"{"search_tool":{"search_tool_name":"x","litellm_params":{}}}"#,
    "missing",
    r#"["body","search_tool","litellm_params","search_provider"]"#
)]
#[case::name_not_text(
    "POST",
    "/search_tools",
    r#"{"search_tool":{"search_tool_name":5,"litellm_params":{"search_provider":"tavily"}}}"#,
    "value_error",
    r#"["body","search_tool","search_tool_name"]"#
)]
#[case::no_litellm_params(
    "POST",
    "/search_tools/test_connection",
    "{}",
    "missing",
    r#"["body","litellm_params"]"#
)]
#[tokio::test]
async fn routes_invalid_body_is_422_before_the_database_gate(
    #[case] method: &str,
    #[case] uri: &str,
    #[case] body: &str,
    #[case] issue_type: &str,
    #[case] loc: &str,
) {
    let mut setup = fixture();
    setup.database = false;
    let harness = build(setup);
    let (status, text) = send(&harness, method, uri, body, Some(master())).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let answer = parse(&text);
    let detail = &answer["detail"][0];
    assert_eq!(detail["type"], issue_type);
    assert_eq!(detail["loc"], parse(loc));
}

#[rstest]
#[case::put_on_list("PUT", "/search_tools/list", UPDATE_BODY, "list")]
#[case::delete_on_list("DELETE", "/search_tools/list", "", "list")]
#[case::get_on_test_connection("GET", "/search_tools/test_connection", "", "test_connection")]
#[case::put_on_test_connection(
    "PUT",
    "/search_tools/test_connection",
    UPDATE_BODY,
    "test_connection"
)]
#[case::delete_on_test_connection("DELETE", "/search_tools/test_connection", "", "test_connection")]
#[tokio::test]
async fn routes_fixed_words_read_as_ids(
    #[case] method: &str,
    #[case] uri: &str,
    #[case] body: &str,
    #[case] word: &str,
) {
    let harness = build(fixture());
    let (status, text) = send(&harness, method, uri, body, Some(master())).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        parse(&text)["detail"],
        format!("Search tool with ID {word} not found")
    );
    assert_eq!(
        *harness.table.calls.lock().unwrap(),
        vec![format!("find_unique:{word}")]
    );
}

#[rstest]
#[case::post_on_list("POST", "/search_tools/list")]
#[case::get_on_collection("GET", "/search_tools")]
#[case::post_on_an_id("POST", "/search_tools/tool-1")]
#[tokio::test]
async fn routes_other_methods_are_405(#[case] method: &str, #[case] uri: &str) {
    let harness = build(fixture());
    let (status, _text) = send(&harness, method, uri, "", Some(master())).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
}

fn listed_fixture() -> Fixture {
    let mut setup = fixture();
    setup.config = Some(vec![
        config_tool(
            r#"{"search_tool_name":"cfg-a","litellm_params":{"search_provider":"tavily","api_key":"tvly-abcdef123","api_base":"https://tavily.example"},"search_tool_info":{"description":"from file"}}"#,
        ),
        config_tool(r#"{"search_tool_name":"","litellm_params":{"search_provider":"tavily"}}"#),
        config_tool(
            r#"{"search_tool_name":"shared","litellm_params":{"search_provider":"exa_ai"}}"#,
        ),
    ]);
    setup.rows = vec![
        record(
            "tool-1",
            "shared",
            r#"{"search_provider":"brave","api_key":"abcd"}"#,
        ),
        record(
            "tool-2",
            "db-b",
            r#"{"search_provider":"serper","api_key":"abcde","max_retries":3}"#,
        ),
    ];
    setup
}

#[tokio::test]
async fn list_merges_config_first_and_masks_every_item() {
    let harness = build(listed_fixture());
    let (status, text) = send(&harness, "GET", "/search_tools/list", "", Some(master())).await;
    assert_eq!(status, StatusCode::OK);
    let expected = parse(&format!(
        r#"{{"search_tools":[
            {{"search_tool_id":null,"search_tool_name":"cfg-a","litellm_params":{{"search_provider":"tavily","api_key":"tv****23","api_base":"https://tavily.example"}},"search_tool_info":{{"description":"from file"}},"created_at":null,"updated_at":null,"is_from_config":true}},
            {{"search_tool_id":"tool-1","search_tool_name":"shared","litellm_params":{{"search_provider":"brave","api_key":"*****"}},"search_tool_info":{{"description":"stored"}},"created_at":"{created}","updated_at":"{updated}","is_from_config":false}},
            {{"search_tool_id":"tool-2","search_tool_name":"db-b","litellm_params":{{"search_provider":"serper","api_key":"ab****de","max_retries":3}},"search_tool_info":{{"description":"stored"}},"created_at":"{created}","updated_at":"{updated}","is_from_config":false}}
        ]}}"#,
        created = isoformat(moment(1_770_000_000)),
        updated = isoformat(moment(1_775_000_000)),
    ));
    assert_eq!(parse(&text), expected);
    assert_eq!(
        *harness.table.calls.lock().unwrap(),
        vec!["find_many:Some(Descending)".to_owned()]
    );
}

#[tokio::test]
async fn list_config_without_search_tool_info_is_null() {
    let mut setup = fixture();
    setup.config = Some(vec![config_tool(
        r#"{"search_tool_name":"bare","litellm_params":{"search_provider":"tavily"}}"#,
    )]);
    let harness = build(setup);
    let (_status, text) = send(&harness, "GET", "/search_tools/list", "", Some(master())).await;
    assert_eq!(
        parse(&text)["search_tools"][0]["search_tool_info"],
        JsonValue::Null
    );
}

#[tokio::test]
async fn list_unreadable_config_lists_the_database_rows() {
    let mut setup = listed_fixture();
    setup.config = None;
    let harness = build(setup);
    let (captured, _guard) = capture();
    let (status, text) = send(&harness, "GET", "/search_tools/list", "", Some(master())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(names(&text), vec!["shared", "db-b"]);
    let logs = log_text(&captured);
    assert!(logs.contains("DEBUG"), "{logs}");
    assert!(
        logs.contains("Could not get config-defined search tools: the config file is unreadable"),
        "{logs}"
    );
}

#[rstest]
#[case::master_key(master())]
#[case::proxy_admin_key(key_caller(r#"{"token":"k1","user_role":"proxy_admin","object_permission":{"object_permission_id":"p1","search_tools":["nothing"]}}"#))]
#[case::proxy_admin_viewer_key(key_caller(r#"{"token":"k2","user_role":"proxy_admin_viewer","object_permission":{"object_permission_id":"p2","search_tools":["nothing"]}}"#))]
#[case::platform_admin_role(caller(r#"{"principal_type":"human","subject":"sso-admin","auth_method":"oidc","roles":["platform_admin"]}"#))]
#[case::platform_viewer_role(caller(r#"{"principal_type":"human","subject":"sso-viewer","auth_method":"oidc","roles":["platform_viewer"]}"#))]
#[tokio::test]
async fn list_admins_see_every_tool(#[case] who: AuthContext) {
    let mut setup = listed_fixture();
    // only an admin sees an unnamed row, so the row tells an admin from an unrestricted key
    setup
        .rows
        .push(record("tool-3", "", r#"{"search_provider":"brave"}"#));
    let harness = build(setup);
    let (_status, text) = send(&harness, "GET", "/search_tools/list", "", Some(who)).await;
    assert_eq!(names(&text), vec!["cfg-a", "shared", "db-b", ""]);
    assert!(harness.teams.lookups.lock().unwrap().is_empty());
}

#[tokio::test]
async fn list_key_allowlist_scopes_a_non_admin() {
    let harness = build(listed_fixture());
    let who = key_caller(
        r#"{"token":"k3","user_role":"internal_user","object_permission":{"object_permission_id":"p3","search_tools":["db-b"]}}"#,
    );
    let (status, text) = send(&harness, "GET", "/search_tools/list", "", Some(who)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(names(&text), vec!["db-b"]);
}

#[tokio::test]
async fn list_caller_without_a_key_record_sees_unrestricted_tools() {
    let harness = build(listed_fixture());
    let who = caller(r#"{"principal_type":"human","subject":"someone","auth_method":"oidc"}"#);
    let (_status, text) = send(&harness, "GET", "/search_tools/list", "", Some(who)).await;
    assert_eq!(names(&text), vec!["cfg-a", "shared", "db-b"]);
}

#[tokio::test]
async fn list_team_allowlist_is_read_once_and_applied() {
    let mut setup = listed_fixture();
    setup.team = TeamAnswer::Row(TEAM_ONE_ALLOWS_CONFIG);
    let harness = build(setup);
    let who = key_caller(r#"{"token":"k4","team_id":"team-1"}"#);
    let (status, text) = send(&harness, "GET", "/search_tools/list", "", Some(who)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(names(&text), vec!["cfg-a"]);
    assert_eq!(*harness.teams.lookups.lock().unwrap(), vec!["team-1"]);
}

#[rstest]
#[case::dashboard_sentinel(r#"{"token":"k5","team_id":"litellm-dashboard"}"#)]
#[case::empty_team_id(r#"{"token":"k6","team_id":""}"#)]
#[case::no_team(r#"{"token":"k7"}"#)]
#[tokio::test]
async fn list_dashboard_sentinel_is_never_looked_up(#[case] fields: &str) {
    let harness = build(listed_fixture());
    let (status, text) = send(
        &harness,
        "GET",
        "/search_tools/list",
        "",
        Some(key_caller(fields)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(names(&text), vec!["cfg-a", "shared", "db-b"]);
    assert!(harness.teams.lookups.lock().unwrap().is_empty());
}

#[rstest]
#[case::team_not_found(TeamAnswer::Missing, StatusCode::NOT_FOUND, r#"{"detail":{"error":"Team doesn't exist in db. Team=team-9. Create team via `/team/new` call."}}"#, false)]
#[case::no_database(
    TeamAnswer::NoDatabase,
    StatusCode::INTERNAL_SERVER_ERROR,
    r#"{"detail":"No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys"}"#,
    true
)]
#[tokio::test]
async fn list_failed_team_lookup_surfaces(
    #[case] answer: TeamAnswer,
    #[case] expected_status: StatusCode,
    #[case] expected_body: &str,
    #[case] logs_error: bool,
) {
    let mut setup = listed_fixture();
    setup.team = answer;
    let harness = build(setup);
    let (captured, _guard) = capture();
    let who = key_caller(r#"{"token":"k8","team_id":"team-9"}"#);
    let (status, text) = send(&harness, "GET", "/search_tools/list", "", Some(who)).await;
    assert_eq!(status, expected_status);
    assert_eq!(parse(&text), parse(expected_body));
    assert_eq!(
        log_text(&captured).contains("Error getting search tools"),
        logs_error
    );
}

#[tokio::test]
async fn list_unnamed_database_row_is_hidden_from_a_non_admin() {
    let mut setup = fixture();
    setup.rows = vec![
        record("tool-1", "", r#"{"search_provider":"brave"}"#),
        record("tool-2", "named", r#"{"search_provider":"brave"}"#),
    ];
    let harness = build(setup);
    let (_status, admin_text) =
        send(&harness, "GET", "/search_tools/list", "", Some(master())).await;
    assert_eq!(names(&admin_text), vec!["", "named"]);
    let who = key_caller(r#"{"token":"k9"}"#);
    let (_status, user_text) = send(&harness, "GET", "/search_tools/list", "", Some(who)).await;
    assert_eq!(names(&user_text), vec!["named"]);
}

#[tokio::test]
async fn list_registry_failure_answers_500_with_the_text() {
    let harness = build(listed_fixture());
    harness.table.broken.store(true, Ordering::SeqCst);
    let (status, text) = send(&harness, "GET", "/search_tools/list", "", Some(master())).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    let detail = parse(&text)["detail"].as_str().unwrap().to_owned();
    assert!(
        detail.starts_with("Error getting search tools from DB: "),
        "{detail}"
    );
}

#[tokio::test]
async fn write_create_returns_the_unmasked_row_and_refreshes() {
    let harness = build(fixture());
    let (captured, _guard) = capture();
    let (status, text) = send(
        &harness,
        "POST",
        "/search_tools",
        CREATE_BODY,
        Some(master()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let stamp = isoformat(fixed_now());
    let expected = parse(&format!(
        r#"{{"search_tool_id":"tool-new","search_tool_name":"fresh","litellm_params":{{"search_provider":"tavily","api_key":"sk-plain-secret"}},"search_tool_info":{{"description":"new"}},"created_at":"{stamp}","updated_at":"{stamp}"}}"#
    ));
    assert_eq!(parse(&text), expected);
    assert_eq!(harness.refresh.calls.load(Ordering::SeqCst), 1);
    assert!(
        log_text(&captured).contains("Successfully added search tool 'fresh' to database."),
        "{}",
        log_text(&captured)
    );
}

#[tokio::test]
async fn write_update_returns_the_stored_row_unmasked() {
    let mut setup = fixture();
    setup.rows = vec![record("tool-1", "old", r#"{"search_provider":"tavily"}"#)];
    let harness = build(setup);
    let (status, text) = send(
        &harness,
        "PUT",
        "/search_tools/tool-1",
        UPDATE_BODY,
        Some(master()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let expected = parse(&format!(
        r#"{{"search_tool_id":"tool-1","search_tool_name":"renamed","litellm_params":{{"search_provider":"brave","api_key":"sk-new-secret-key"}},"search_tool_info":{{}},"created_at":"{created}","updated_at":"{updated}"}}"#,
        created = isoformat(moment(1_770_000_000)),
        updated = isoformat(fixed_now()),
    ));
    assert_eq!(parse(&text), expected);
    assert_eq!(harness.refresh.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn write_delete_returns_the_message_and_name() {
    let mut setup = fixture();
    setup.rows = vec![record("tool-1", "gone", r#"{"search_provider":"tavily"}"#)];
    let harness = build(setup);
    let (status, text) = send(
        &harness,
        "DELETE",
        "/search_tools/tool-1",
        "",
        Some(master()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        parse(&text),
        parse(r#"{"message":"Search tool tool-1 deleted successfully","search_tool_name":"gone"}"#)
    );
    assert_eq!(harness.refresh.calls.load(Ordering::SeqCst), 1);
    assert!(harness.table.rows.lock().unwrap().is_empty());
}

#[rstest]
#[case::update("PUT", UPDATE_BODY)]
#[case::delete("DELETE", "")]
#[case::info("GET", "")]
#[tokio::test]
async fn write_missing_id_is_404(#[case] method: &str, #[case] body: &str) {
    let harness = build(fixture());
    let (status, text) = send(
        &harness,
        method,
        "/search_tools/absent",
        body,
        Some(master()),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        parse(&text)["detail"],
        "Search tool with ID absent not found"
    );
    assert_eq!(harness.refresh.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn write_get_masks_params_and_marks_the_database_origin() {
    let mut setup = fixture();
    setup.rows = vec![record(
        "tool-1",
        "kept",
        r#"{"search_provider":"tavily","api_key":"tvly-secret-value","api_base":"https://tavily.example"}"#,
    )];
    let harness = build(setup);
    let (status, text) = send(&harness, "GET", "/search_tools/tool-1", "", Some(master())).await;
    assert_eq!(status, StatusCode::OK);
    let expected = parse(&format!(
        r#"{{"search_tool_id":"tool-1","search_tool_name":"kept","litellm_params":{{"search_provider":"tavily","api_key":"tv****ue","api_base":"https://tavily.example"}},"search_tool_info":{{"description":"stored"}},"created_at":"{created}","updated_at":"{updated}","is_from_config":false}}"#,
        created = isoformat(moment(1_770_000_000)),
        updated = isoformat(moment(1_775_000_000)),
    ));
    assert_eq!(parse(&text), expected);
}

#[rstest]
#[case::create("POST", "/search_tools", CREATE_BODY)]
#[case::update("PUT", "/search_tools/tool-1", UPDATE_BODY)]
#[case::delete("DELETE", "/search_tools/tool-1", "")]
#[tokio::test]
async fn write_failing_refresh_keeps_the_normal_answer(
    #[case] method: &str,
    #[case] uri: &str,
    #[case] body: &str,
) {
    let mut setup = fixture();
    setup.rows = vec![record("tool-1", "old", r#"{"search_provider":"tavily"}"#)];
    setup.refresh_fails = true;
    let harness = build(setup);
    let (captured, _guard) = capture();
    let (status, _text) = send(&harness, method, uri, body, Some(master())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(harness.refresh.calls.load(Ordering::SeqCst), 1);
    let logs = log_text(&captured);
    assert!(logs.contains("ERROR"), "{logs}");
    assert!(
        logs.contains(
            "Search tool router refresh failed after a management write: the router reload failed"
        ),
        "{logs}"
    );
}

#[rstest]
#[case::create(
    "POST",
    "/search_tools",
    CREATE_BODY,
    "Error adding search tool to DB: "
)]
#[case::update(
    "PUT",
    "/search_tools/tool-1",
    UPDATE_BODY,
    "Error getting search tool from DB: "
)]
#[case::delete(
    "DELETE",
    "/search_tools/tool-1",
    "",
    "Error getting search tool from DB: "
)]
#[case::info(
    "GET",
    "/search_tools/tool-1",
    "",
    "Error getting search tool from DB: "
)]
#[tokio::test]
async fn write_failing_registry_answers_500_with_the_text(
    #[case] method: &str,
    #[case] uri: &str,
    #[case] body: &str,
    #[case] prefix: &str,
) {
    let mut setup = fixture();
    setup.rows = vec![record("tool-1", "old", r#"{"search_provider":"tavily"}"#)];
    let harness = build(setup);
    harness.table.broken.store(true, Ordering::SeqCst);
    let (status, text) = send(&harness, method, uri, body, Some(master())).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    let detail = parse(&text)["detail"].as_str().unwrap().to_owned();
    assert!(detail.starts_with(prefix), "{detail}");
    assert!(detail.ends_with("the table is down"), "{detail}");
    assert_eq!(harness.refresh.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn connection_success_calls_asearch_with_the_test_query() {
    let harness = build(fixture());
    let (status, text) = send(
        &harness,
        "POST",
        "/search_tools/test_connection",
        CONNECT_BODY,
        Some(master()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        parse(&text),
        parse(
            r#"{"status":"success","message":"Successfully connected to tavily search provider","test_query":"test","results_count":2}"#
        )
    );
    let requests = harness.search.requests.lock().unwrap();
    let [request] = requests.as_slice() else {
        panic!("one search call, got {}", requests.len());
    };
    assert!(matches!(&request.query, Json::Text(query) if query == "test"));
    assert_eq!(request.search_provider, "tavily");
    assert_eq!(request.options.max_results, Some(1));
    assert_eq!(request.timeout, Some(Duration::from_secs(10)));
    assert_eq!(
        request
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        Some("tvly-test-key".to_owned())
    );
    assert_eq!(request.api_base.as_deref(), Some("https://search.example"));
    assert!(request.kwargs.is_empty());
    assert!(request.extra_headers.is_empty());
}

#[tokio::test]
async fn connection_without_results_counts_zero() {
    let mut setup = fixture();
    setup.search = Ok(0);
    let harness = build(setup);
    let body = r#"{"litellm_params":{"search_provider":"brave"}}"#;
    let (_status, text) = send(
        &harness,
        "POST",
        "/search_tools/test_connection",
        body,
        Some(master()),
    )
    .await;
    assert_eq!(parse(&text)["results_count"], 0_u64);
    let requests = harness.search.requests.lock().unwrap();
    assert!(
        requests
            .iter()
            .all(|request| request.api_key.is_none() && request.api_base.is_none())
    );
}

#[tokio::test]
async fn connection_failure_answers_200_with_the_error_class() {
    let mut setup = fixture();
    setup.search = Err((
        "AuthenticationError",
        "litellm.AuthenticationError: the key was refused",
    ));
    let harness = build(setup);
    let (captured, _guard) = capture();
    let (status, text) = send(
        &harness,
        "POST",
        "/search_tools/test_connection",
        CONNECT_BODY,
        Some(master()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        parse(&text),
        parse(
            r#"{"status":"error","message":"litellm.AuthenticationError: the key was refused","error_type":"AuthenticationError"}"#
        )
    );
    let logs = log_text(&captured);
    assert!(logs.contains("ERROR"), "{logs}");
    assert!(
        logs.contains("Failed to connect to search provider"),
        "{logs}"
    );
}

#[rstest]
#[case::absent(r#"{"litellm_params":{"api_key":"sk-x"}}"#)]
#[case::null(r#"{"litellm_params":{"search_provider":null}}"#)]
#[case::empty(r#"{"litellm_params":{"search_provider":""}}"#)]
#[tokio::test]
async fn connection_missing_provider_is_an_http_exception_answer(#[case] body: &str) {
    let harness = build(fixture());
    let (status, text) = send(
        &harness,
        "POST",
        "/search_tools/test_connection",
        body,
        Some(master()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let expected = format!(
        r#"{{"status":"error","message":"{MISSING_PROVIDER}","error_type":"HTTPException"}}"#
    );
    assert_eq!(parse(&text), parse(&expected));
    assert!(harness.search.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn providers_follow_the_enum_order_and_skip_unusable_ones() {
    let harness = build(fixture());
    let (captured, _guard) = capture();
    let (status, text) = send(
        &harness,
        "GET",
        "/search_tools/ui/available_providers",
        "",
        Some(master()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let listed: Vec<(String, String)> = parse(&text)["providers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|provider| {
            (
                provider["provider_name"].as_str().unwrap().to_owned(),
                provider["ui_friendly_name"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let expected: Vec<(String, String)> = [
        "perplexity",
        "parallel_ai",
        "exa_ai",
        "google_pse",
        "dataforseo",
        "firecrawl",
        "fastcrw",
        "searxng",
        "linkup",
        "duckduckgo",
        "searchapi",
        "serper",
        "you_com",
        "apiserpent",
        "tinyfish",
        "agentcore",
        "nimble",
        "bing_grounding",
    ]
    .iter()
    .map(|name| ((*name).to_owned(), format!("UI {name}")))
    .collect();
    assert_eq!(listed, expected);
    let logs = log_text(&captured);
    assert!(
        logs.contains(
            "Could not get config for search provider brave: the brave config failed to build"
        ),
        "{logs}"
    );
}

#[test]
fn providers_list_every_search_provider_value_in_order() {
    assert_eq!(
        SEARCH_PROVIDERS,
        [
            "perplexity",
            "tavily",
            "parallel_ai",
            "exa_ai",
            "brave",
            "google_pse",
            "dataforseo",
            "firecrawl",
            "fastcrw",
            "searxng",
            "linkup",
            "duckduckgo",
            "searchapi",
            "serper",
            "you_com",
            "apiserpent",
            "tinyfish",
            "agentcore",
            "nimble",
            "bing_grounding",
        ]
    );
}

#[rstest]
#[case::list("GET", "/search_tools/list", "", 0)]
#[case::create("POST", "/search_tools", CREATE_BODY, 1)]
#[case::update("PUT", "/search_tools/tool-1", UPDATE_BODY, 1)]
#[case::info("GET", "/search_tools/tool-1", "", 0)]
#[case::test_connection("POST", "/search_tools/test_connection", CONNECT_BODY, 0)]
#[case::providers("GET", "/search_tools/ui/available_providers", "", 0)]
#[case::delete("DELETE", "/search_tools/tool-1", "", 1)]
#[tokio::test]
async fn oneshot_each_route_answers_through_the_module_router(
    #[case] method: &str,
    #[case] uri: &str,
    #[case] body: &str,
    #[case] refreshes: usize,
) {
    let mut setup = fixture();
    setup.rows = vec![record(
        "tool-1",
        "seeded",
        r#"{"search_provider":"tavily"}"#,
    )];
    let harness = build(setup);
    let (status, text) = send(&harness, method, uri, body, Some(master())).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(harness.refresh.calls.load(Ordering::SeqCst), refreshes);
}
