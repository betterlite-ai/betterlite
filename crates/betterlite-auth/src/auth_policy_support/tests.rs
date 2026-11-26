#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::database_availability::policy::{AllowRequestsOnOutage, PolicyError};
use betterlite_db::team_rows::team::{LiteLLMTeamTable, Nullable};
use betterlite_error::error::details::BudgetExceededDetails;
use betterlite_error::proxy_exception::ProxyException;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;
use rust_decimal::Decimal;

use super::extract::{
    ExtractionError, ToolNameExtractors, anthropic_tool_names, extract_request_tool_names,
};
use super::fetch::safe_fetch;
use super::guardrails::{
    GUARDRAIL_DENIAL, GuardrailCheckError, can_modify_guardrails, guardrail_modification_check,
};
use super::org::inherit_org_identity;
use super::team::{TeamFromTokenError, UI_TEAM_ID, team_obj_from_token, token_can_vouch_for_team};
use super::tools::{ToolCheckError, check_tools_allowlist as check};
use crate::auth_end_users::check::EndUserBudgetExceeded;
use crate::auth_org_lookup::error::OrgLookupError;
use crate::auth_org_lookup::organization::OrgLookup;
use crate::auth_team_lookup::error::{TeamFailure, TeamLookupError};
use crate::resolved_key::auth::UserAPIKeyAuth;

mod support;

use support::{
    Logs, body, key, org_lookup, org_lookup_without_database, org_row, org_world,
    proxy_exception_denial, routes, team,
};

const ROUTE: &str = "/v1/chat/completions";

fn chat_extractors() -> ToolNameExtractors {
    let table = routes(&[(ROUTE, &["acompletion", "completion"])]);
    ToolNameExtractors::builtin(table)
}

fn tools_token(key_allowed: &str, team_allowed: &str) -> UserAPIKeyAuth {
    let mut fields = String::from(r#"{"token":"k","metadata":{"allowed_tools":"#);
    fields.push_str(key_allowed);
    fields.push_str(r#"},"team_metadata":{"allowed_tools":"#);
    fields.push_str(team_allowed);
    fields.push_str("}}");
    key(&fields)
}

fn chat_request(name: &str) -> JsonObject {
    let text = format!(r#"{{"tools":[{{"type":"function","function":{{"name":"{name}"}}}}]}}"#);
    body(&text)
}

fn denial_of(outcome: Result<(), ToolCheckError>) -> ProxyException {
    match outcome {
        Err(ToolCheckError::Denied(exception)) => *exception,
        Err(ToolCheckError::Extraction(error)) => panic!("unexpected extraction error: {error}"),
        Ok(()) => panic!("the tool check was expected to deny the request"),
    }
}

fn denying_team() -> LiteLLMTeamTable {
    team(r#"{"team_id":"t1","metadata":{"guardrails":{"modify_guardrails":false}}}"#)
}

fn assert_guardrail_denied(request: &str) {
    let refused = guardrail_modification_check(&body(request), Some(&denying_team()));
    assert!(
        matches!(refused, Err(GuardrailCheckError::Denied)),
        "{request}"
    );
}

fn assert_can_modify(fields: &[&str], allowed: bool) {
    for entry in fields {
        assert_eq!(
            can_modify_guardrails(Some(&team(entry))),
            allowed,
            "{entry}"
        );
    }
}

fn budget_details() -> BudgetExceededDetails {
    BudgetExceededDetails {
        current_cost: Decimal::ZERO,
        max_budget: Decimal::ONE,
        message: Some("budget exceeded".to_owned()),
        provider: String::new(),
        entity_type: None,
        entity_id: None,
        status_code: 429,
    }
}

async fn inherit(
    token: &mut UserAPIKeyAuth,
    lookup: &OrgLookup,
    team_row: Option<&LiteLLMTeamTable>,
) {
    inherit_org_identity(
        token,
        team_row,
        lookup,
        &AllowRequestsOnOutage::Boolean(false),
    )
    .await
    .unwrap();
}

#[rstest]
#[case(r#"{"metadata":{"guardrails":false}}"#)]
#[case(r#"{"metadata":{"disable_global_guardrails":false}}"#)]
#[case(r#"{"metadata":{"disable_global_guardrail":false}}"#)]
#[case(r#"{"metadata":{"opted_out_global_guardrails":false}}"#)]
#[case(r#"{"litellm_metadata":{"guardrails":null}}"#)]
#[case(r#"{"litellm_metadata":{"disable_global_guardrails":null}}"#)]
#[case(r#"{"litellm_metadata":{"disable_global_guardrail":null}}"#)]
#[case(r#"{"litellm_metadata":{"opted_out_global_guardrails":null}}"#)]
#[case(r#"{"guardrails":[]}"#)]
#[case(r#"{"disable_global_guardrails":{}}"#)]
#[case(r#"{"disable_global_guardrail":""}"#)]
#[case(r#"{"opted_out_global_guardrails":0}"#)]
fn guardrail_four_keys_three_locations(#[case] request: &str) {
    assert_guardrail_denied(request);
}

#[rstest]
#[case(r#"{"metadata":{"disable_global_guardrails":false}}"#)]
#[case(r#"{"metadata":{"disable_global_guardrails":null}}"#)]
#[case(r#"{"litellm_metadata":{"disable_global_guardrails":false}}"#)]
#[case(r#"{"litellm_metadata":{"disable_global_guardrails":null}}"#)]
#[case(r#"{"opted_out_global_guardrails":false}"#)]
#[case(r#"{"opted_out_global_guardrails":null}"#)]
fn guardrail_false_and_null_count(#[case] request: &str) {
    assert_guardrail_denied(request);
}

#[rstest]
#[case(
    r#"{"metadata":"{\"guardrails\":{\"modify_guardrails\":false}}"}"#,
    true
)]
#[case(r#"{"litellm_metadata":"{\"disable_global_guardrail\":false}"}"#, true)]
#[case(r#"{"metadata":"{not json"}"#, false)]
#[case(r#"{"metadata":"[1, 2]"}"#, false)]
#[case(r#"{"metadata":"null"}"#, false)]
#[case(r#"{"metadata":"17"}"#, false)]
#[case(r#"{"metadata":"\"text\""}"#, false)]
#[case(r#"{"metadata":17}"#, false)]
#[case(r#"{"metadata":[]}"#, false)]
#[case(r#"{"litellm_metadata":false}"#, false)]
fn guardrail_json_object_and_malformed(#[case] request: &str, #[case] denied: bool) {
    if denied {
        assert_guardrail_denied(request);
    } else {
        guardrail_modification_check(&body(request), Some(&denying_team())).unwrap();
    }
}

#[test]
fn modify_false_only_denies() {
    assert_can_modify(
        &[r#"{"team_id":"t","metadata":{"guardrails":{"modify_guardrails":false}}}"#],
        false,
    );
    assert_can_modify(
        &[
            r#"{"team_id":"t","metadata":{"guardrails":{"modify_guardrails":true}}}"#,
            r#"{"team_id":"t","metadata":{"guardrails":{"modify_guardrails":"false"}}}"#,
            r#"{"team_id":"t","metadata":{"guardrails":{"modify_guardrails":0}}}"#,
            r#"{"team_id":"t","metadata":{"guardrails":{"modify_guardrails":null}}}"#,
            r#"{"team_id":"t","metadata":{"guardrails":{}}}"#,
            r#"{"team_id":"t","metadata":{"guardrails":["modify_guardrails"]}}"#,
            r#"{"team_id":"t","metadata":{"guardrails":"modify_guardrails"}}"#,
            r#"{"team_id":"t"}"#,
        ],
        true,
    );
    assert!(can_modify_guardrails(None));

    let request = body(r#"{"metadata":{"guardrails":["pii"]}}"#);
    let exception = guardrail_modification_check(&request, Some(&denying_team()))
        .unwrap_err()
        .into_proxy_exception();
    assert_eq!(exception.message, GUARDRAIL_DENIAL);
    assert_eq!(exception.error_type, "auth_error");
    assert_eq!(exception.param.as_deref(), Some("None"));
    assert_eq!(exception.code, "403");
}

#[rstest]
#[case(r#"{"metadata":{"guardrails":["pii"]}}"#)]
#[case(r#"{"litellm_metadata":{"disable_global_guardrails":true}}"#)]
#[case(r#"{"opted_out_global_guardrails":["pii"]}"#)]
fn no_team_allows(#[case] request: &str) {
    guardrail_modification_check(&body(request), None).unwrap();
}

#[test]
fn tool_key_overrides_team() {
    let extractors = chat_extractors();
    let token = tools_token(r#"["key-tool"]"#, r#"["team-tool"]"#);
    let refused = chat_request("team-tool");
    let denial = denial_of(check(&refused, Some(&token), None, ROUTE, &extractors));
    assert!(denial.message.contains("team-tool"), "{}", denial.message);
    assert_eq!(denial.error_type, "tool_access_denied");
    assert_eq!(denial.param.as_deref(), Some("tools"));
    assert_eq!(denial.code, "403");
    let permitted = chat_request("key-tool");
    check(&permitted, Some(&token), None, ROUTE, &extractors).unwrap();
}

// the denial keeps its status and param through the exception conversion the proxy stage uses
#[test]
fn tool_denial_survives_the_proxy_conversion() {
    let extractors = chat_extractors();
    let token = tools_token(r#"["allowed"]"#, "null");
    let refused = chat_request("other");
    let error = check(&refused, Some(&token), None, ROUTE, &extractors).unwrap_err();
    let exception = error.into_proxy_exception();
    assert_eq!(exception.error_type, "tool_access_denied");
    assert_eq!(exception.param.as_deref(), Some("tools"));
    assert_eq!(exception.code, "403");
}

#[rstest]
#[case("[]")]
#[case(r#""not-a-list""#)]
#[case("null")]
#[case("5")]
fn tool_empty_key_falls_back(#[case] key_allowed: &str) {
    let extractors = chat_extractors();
    let permitted = chat_request("team-tool");
    let refused = chat_request("other");
    let token = tools_token(key_allowed, r#"["team-tool"]"#);
    check(&permitted, Some(&token), None, ROUTE, &extractors).unwrap();
    let denial = denial_of(check(&refused, Some(&token), None, ROUTE, &extractors));
    assert_eq!(denial.error_type, "tool_access_denied");
}

#[rstest]
#[case(r#""not-a-list""#, "5")]
#[case(r#""not-a-list""#, "null")]
#[case("[]", "null")]
#[case("null", "{}")]
#[case("{}", r#""text""#)]
fn tool_bad_effective_allows(#[case] key_allowed: &str, #[case] team_allowed: &str) {
    let extractors = chat_extractors();
    let request = chat_request("any-tool");
    let token = tools_token(key_allowed, team_allowed);
    check(&request, Some(&token), None, ROUTE, &extractors).unwrap();
    let bare = key(r#"{"token":"k"}"#);
    check(&request, Some(&bare), None, ROUTE, &extractors).unwrap();
    check(&request, None, None, ROUTE, &extractors).unwrap();
}

#[rstest]
#[case("/v1/unknown")]
#[case("/embed")]
#[case("")]
fn tool_route_skip(#[case] skipped: &str) {
    let table = routes(&[("/embed", &["embedding"]), (ROUTE, &["completion"])]);
    let extractors = ToolNameExtractors::builtin(table);
    let token = tools_token(r#"["allowed"]"#, "null");
    let request = chat_request("other");
    check(&request, Some(&token), None, skipped, &extractors).unwrap();
    let denial = denial_of(check(&request, Some(&token), None, ROUTE, &extractors));
    assert_eq!(denial.error_type, "tool_access_denied");
}

#[test]
fn tool_exact_string_names_and_duplicates() {
    let extractors = chat_extractors();
    let token = tools_token(r#"["Alpha"]"#, "null");
    let request = body(r#"{"functions":[{"name":"Alpha"},{"name":"alpha"},{"name":"Alpha"}]}"#);
    let denial = denial_of(check(&request, Some(&token), None, ROUTE, &extractors));
    assert!(
        denial.message.contains(r#"["alpha", "Alpha"]"#),
        "{}",
        denial.message
    );

    let duplicates = body(r#"{"functions":[{"name":"X"},{"name":"X"}]}"#);
    let refusing = tools_token(r#"["Y"]"#, "null");
    let repeated_denial = denial_of(check(
        &duplicates,
        Some(&refusing),
        None,
        ROUTE,
        &extractors,
    ));
    assert!(
        repeated_denial.message.contains(r#"["X", "X"]"#),
        "{}",
        repeated_denial.message
    );

    let converted = body(r#"{"functions":[{"name":"1"},{"name":"True"}]}"#);
    let numeric = tools_token("[1, true]", "null");
    check(&converted, Some(&numeric), None, ROUTE, &extractors).unwrap();
}

#[test]
fn tool_team_argument_unused() {
    let extractors = chat_extractors();
    let team_argument = team(r#"{"team_id":"t1","metadata":{"allowed_tools":["x"]}}"#);
    let bare = key(r#"{"token":"k"}"#);
    let anything = chat_request("anything");
    check(
        &anything,
        Some(&bare),
        Some(&team_argument),
        ROUTE,
        &extractors,
    )
    .unwrap();
    let token = tools_token(r#"["allowed"]"#, "null");
    let refused = chat_request("x");
    let denial = denial_of(check(
        &refused,
        Some(&token),
        Some(&team_argument),
        ROUTE,
        &extractors,
    ));
    assert_eq!(denial.error_type, "tool_access_denied");
}

#[rstest]
#[case("/c")]
#[case("/ac")]
#[case("/r")]
#[case("/ar")]
#[case("/a")]
#[case("/g")]
#[case("/ag")]
#[case("/m")]
fn tool_all_eight_call_types(#[case] route: &str) {
    let table = routes(&[
        ("/c", &["completion"]),
        ("/ac", &["acompletion"]),
        ("/r", &["responses"]),
        ("/ar", &["aresponses"]),
        ("/a", &["anthropic_messages"]),
        ("/g", &["generate_content"]),
        ("/ag", &["agenerate_content"]),
        ("/m", &["call_mcp_tool"]),
    ]);
    let extractors = ToolNameExtractors::builtin(table);
    let token = tools_token(r#"["x"]"#, "null");
    let request = match route {
        "/c" | "/ac" => r#"{"functions":[{"name":"x"}]}"#,
        "/r" => r#"{"tools":[{"type":"function","name":"x"}]}"#,
        "/ar" => r#"{"tools":[{"type":"custom","name":"x"}]}"#,
        "/a" => r#"{"tools":[{"name":"x"}]}"#,
        "/g" | "/ag" => r#"{"tools":[{"functionDeclarations":[{"name":"x"}]}]}"#,
        "/m" => r#"{"name":"x"}"#,
        _ => "{}",
    };
    check(&body(request), Some(&token), None, route, &extractors).unwrap();
    let other = request.replace("\"x\"", "\"y\"");
    let denial = denial_of(check(&body(&other), Some(&token), None, route, &extractors));
    assert_eq!(denial.error_type, "tool_access_denied");
}

#[test]
fn standalone_empty_stops() {
    let table = routes(&[("/both", &["generate_content", "completion"])]);
    let extractors = ToolNameExtractors::builtin(table);
    let request = chat_request("chat-tool");
    let names = extract_request_tool_names("/both", &request, &extractors).unwrap();
    assert!(names.is_empty());
    let token = tools_token(r#"["nothing"]"#, "null");
    check(&request, Some(&token), None, "/both", &extractors).unwrap();
}

#[test]
fn handler_empty_continues() {
    let table = routes(&[("/both", &["completion", "responses"])]);
    let extractors = ToolNameExtractors::builtin(table);
    let request = body(r#"{"tools":[{"type":"mcp","server_label":"server-x"}]}"#);
    let names = extract_request_tool_names("/both", &request, &extractors).unwrap();
    assert_eq!(names, vec!["server-x".to_owned()]);
    let token = tools_token(r#"["other"]"#, "null");
    let denial = denial_of(check(&request, Some(&token), None, "/both", &extractors));
    assert!(denial.message.contains("server-x"), "{}", denial.message);
}

#[test]
fn missing_handler_empty() {
    let empty = ToolNameExtractors {
        routes: routes(&[("/x", &["completion"])]),
        standalone: BTreeMap::new(),
        handlers: BTreeMap::new(),
    };
    let request = body(r#"{"functions":[{"name":"a"}]}"#);
    let names = extract_request_tool_names("/x", &request, &empty).unwrap();
    assert!(names.is_empty());
    let table = routes(&[("/embed", &["embedding"])]);
    let unsupported = ToolNameExtractors::builtin(table);
    let other_names = extract_request_tool_names("/embed", &request, &unsupported).unwrap();
    assert!(other_names.is_empty());
}

#[test]
fn chat_and_legacy_names() {
    let request = body(
        r#"{"tools":[{"type":"function","function":{"name":"fn-a"}},{"type":"other","function":{"name":"skipped"}},{"type":"function","function":{"name":""}},{"type":"function","function":"not-a-dict"},{"type":"function","function":{"name":"fn-b"}}],"functions":[{"name":"legacy-a"},{"name":false},{"name":"legacy-b"},"flat"]}"#,
    );
    let names = extract_request_tool_names(ROUTE, &request, &chat_extractors()).unwrap();
    assert_eq!(
        names,
        vec![
            "fn-a".to_owned(),
            "fn-b".to_owned(),
            "legacy-a".to_owned(),
            "legacy-b".to_owned(),
        ]
    );
}

#[test]
fn responses_custom_and_mcp_names() {
    let table = routes(&[("/responses", &["responses"])]);
    let extractors = ToolNameExtractors::builtin(table);
    let request = body(
        r#"{"tools":[{"type":"function","name":"fn"},{"type":"custom","name":"custom"},{"type":"mcp","server_label":"server"},{"type":"mcp","server_label":""},{"type":"function","name":""},{"type":"other","name":"skipped"},{"type":"function","name":5}]}"#,
    );
    let names = extract_request_tool_names("/responses", &request, &extractors).unwrap();
    assert_eq!(
        names,
        vec![
            "fn".to_owned(),
            "custom".to_owned(),
            "server".to_owned(),
            "5".to_owned(),
        ]
    );
}

#[test]
fn gemini_declarations() {
    let table = routes(&[("/generate", &["generate_content"])]);
    let extractors = ToolNameExtractors::builtin(table);
    let request = body(
        r#"{"tools":[{"functionDeclarations":[{"name":"a"},{"name":false},{"name":"b"},"flat",{"name":5}]},{"other":true},{"functionDeclarations":"plain"}]}"#,
    );
    let names = extract_request_tool_names("/generate", &request, &extractors).unwrap();
    assert_eq!(names, vec!["a".to_owned(), "b".to_owned(), "5".to_owned()]);
}

#[rstest]
#[case(r#"{"name":"a","mcp_tool_name":"b"}"#, vec!["a"])]
#[case(r#"{"name":"","mcp_tool_name":"b"}"#, vec!["b"])]
#[case(r#"{"name":null,"mcp_tool_name":"b"}"#, vec!["b"])]
#[case(r#"{"mcp_tool_name":"b"}"#, vec!["b"])]
#[case(r#"{"name":"a"}"#, vec!["a"])]
#[case(r#"{"name":false,"mcp_tool_name":0}"#, vec![])]
#[case(r#"{"name":5}"#, vec!["5"])]
fn mcp_name_precedence(#[case] request: &str, #[case] expected: Vec<&str>) {
    let table = routes(&[("/mcp", &["call_mcp_tool"])]);
    let extractors = ToolNameExtractors::builtin(table);
    let names = extract_request_tool_names("/mcp", &body(request), &extractors).unwrap();
    let wanted: Vec<String> = expected.into_iter().map(str::to_owned).collect();
    assert_eq!(names, wanted, "{request}");
}

#[rstest]
#[case(r#"{"name":"flat"}"#, vec!["flat"])]
#[case(
    r#"{"type":"function","name":"flat","function":{"name":"fn"}}"#,
    vec!["flat", "fn"]
)]
#[case(r#"{"type":"function","function":{"name":"fn"}}"#, vec!["fn"])]
#[case(r#"{"name":"","type":"function","function":{"name":"fn"}}"#, vec!["fn"])]
#[case(r#"{"type":"custom","function":{"name":"fn"}}"#, vec![])]
#[case(r#"{"name":5}"#, vec![])]
#[case(r#"{"function":{"name":"fn"}}"#, vec![])]
fn anthropic_both_names(#[case] request: &str, #[case] expected: Vec<&str>) {
    let table = routes(&[("/anthropic", &["anthropic_messages"])]);
    let extractors = ToolNameExtractors::builtin(table);
    let names = extract_request_tool_names("/anthropic", &body(request), &extractors).unwrap();
    let wanted: Vec<String> = expected.into_iter().map(str::to_owned).collect();
    assert_eq!(names, wanted, "{request}");
    assert!(anthropic_tool_names(&Json::Null).is_empty());
    let mixed = body(r#"{"name":"flat","type":"function","function":"not-a-dict"}"#);
    let direct = anthropic_tool_names(&Json::Object(mixed));
    assert_eq!(direct, vec!["flat".to_owned()]);
}

#[test]
fn extraction_falsey_names_and_wrong_objects() {
    let request = body(
        r#"{"tools":[false,null,0,"text",[1],{"type":"function","function":{"name":false}},{"type":"function","function":{"name":null}},{"type":"function","function":{"name":0}},{"type":"function","function":{"name":""}},{"type":"function"},{"type":"function","function":[1]}],"functions":[false,{"name":0},{"name":"kept"}]}"#,
    );
    let names = extract_request_tool_names(ROUTE, &request, &chat_extractors()).unwrap();
    assert_eq!(names, vec!["kept".to_owned()]);
    let table = routes(&[("/responses", &["responses"])]);
    let responses = ToolNameExtractors::builtin(table);
    let response_request =
        body(r#"{"tools":[{"type":"mcp","server_label":false},{"type":"mcp"}]}"#);
    let other_names =
        extract_request_tool_names("/responses", &response_request, &responses).unwrap();
    assert!(other_names.is_empty());
}

#[rstest]
#[case(ROUTE, r#"{"tools":5}"#, "tools")]
#[case(ROUTE, r#"{"tools":true}"#, "tools")]
#[case(
    "/generate",
    r#"{"tools":[{"functionDeclarations":5}]}"#,
    "functionDeclarations"
)]
fn extraction_noniterable_error(
    #[case] route: &str,
    #[case] request: &str,
    #[case] field: &'static str,
) {
    let extractors = if route == ROUTE {
        chat_extractors()
    } else {
        let table = routes(&[("/generate", &["generate_content"])]);
        ToolNameExtractors::builtin(table)
    };
    assert!(matches!(
        extract_request_tool_names(route, &body(request), &extractors),
        Err(ExtractionError::NotIterable { field: found }) if found == field
    ));
}

#[rstest]
#[case(r#"{"tools":0}"#)]
#[case(r#"{"tools":false}"#)]
#[case(r#"{"tools":""}"#)]
#[case(r#"{"tools":{}}"#)]
#[case(r#"{"tools":{"a":1}}"#)]
#[case(r#"{"tools":"text"}"#)]
fn extraction_falsey_values_empty(#[case] request: &str) {
    let names = extract_request_tool_names(ROUTE, &body(request), &chat_extractors()).unwrap();
    assert!(names.is_empty());
}

#[test]
fn tool_check_wraps_extraction_error() {
    let extractors = chat_extractors();
    let token = tools_token(r#"["allowed"]"#, "null");
    let scalar = body(r#"{"tools":5}"#);
    let rejected = check(&scalar, Some(&token), None, ROUTE, &extractors);
    assert!(matches!(rejected, Err(ToolCheckError::Extraction(_))));
    let exception = ExtractionError::NotIterable { field: "tools" }.into_proxy_exception();
    assert_eq!(exception.error_type, "auth_error");
    assert_eq!(exception.param.as_deref(), Some("None"));
    assert_eq!(exception.code, "401");
}

#[tokio::test]
async fn safe_fetch_rethrows_three_auth_errors() {
    let logs = Logs::default();
    let _guard = logs.install();
    let team_failure = safe_fetch("team", async {
        Err::<(), TeamLookupError>(TeamLookupError::TeamNotFound {
            team_id: "t1".to_owned(),
        })
    })
    .await;
    assert!(matches!(
        team_failure,
        Err(TeamLookupError::TeamNotFound { .. })
    ));
    let proxy_failure = safe_fetch("proxy", async {
        Err::<(), ProxyException>(proxy_exception_denial())
    })
    .await;
    assert_eq!(proxy_failure.unwrap_err().code, "403");
    let budget_failure = safe_fetch("end_user", async {
        Err::<(), EndUserBudgetExceeded>(EndUserBudgetExceeded {
            details: Box::new(budget_details()),
        })
    })
    .await;
    assert!(budget_failure.unwrap_err().details.message.is_some());
    let seen = logs.seen();
    assert_eq!(
        seen.iter()
            .filter(|line| line.contains("fetch failed"))
            .count(),
        3
    );
    assert!(!seen.iter().any(|line| line.contains("swallowed")));
}

#[tokio::test]
async fn safe_fetch_other_debug_drop() {
    let logs = Logs::default();
    let _guard = logs.install();
    let dropped = safe_fetch("team", async {
        Err::<u8, TeamLookupError>(TeamLookupError::NoDatabase)
    })
    .await
    .unwrap();
    assert!(dropped.is_none());
    let kept = safe_fetch("team", async { Ok::<u8, TeamLookupError>(5) }).await;
    assert_eq!(kept.unwrap(), Some(5));
    let seen = logs.seen();
    assert_eq!(
        seen.iter()
            .filter(|line| line.contains("fetch swallowed"))
            .count(),
        1
    );
    assert!(!seen.iter().any(|line| line.contains("fetch failed")));
}

// the org lookup's HTTPException statuses re-raise, and its plain failures are swallowed
#[tokio::test]
async fn safe_fetch_org_lookup_both_sides() {
    let raised = safe_fetch("org", async {
        Err::<(), OrgLookupError>(OrgLookupError::AliasNotFound {
            org_alias: "a".to_owned(),
        })
    })
    .await;
    assert!(matches!(raised, Err(OrgLookupError::AliasNotFound { .. })));
    let dropped = safe_fetch("org", async {
        Err::<u8, OrgLookupError>(OrgLookupError::NoDatabase)
    })
    .await
    .unwrap();
    assert!(dropped.is_none());
}

#[test]
fn reconstructed_team_all_fields() {
    let token = key(
        r#"{"token":"k","team_id":"t1","team_max_budget":10,"team_soft_budget":5,"team_model_max_budget":{"gpt-4":2},"team_spend":1.5,"team_tpm_limit":10,"team_rpm_limit":20,"team_tpd_limit":30,"team_blocked":true,"team_models":["gpt-4"],"team_metadata":{"k":"v"},"team_object_permission_id":"perm"}"#,
    );
    let cached = team_obj_from_token(&token).unwrap();
    let fields = &cached.team.fields;
    assert_eq!(
        (
            fields.base.team_id.as_str(),
            fields.base.max_budget,
            fields.base.soft_budget,
            fields.spend,
        ),
        (
            "t1",
            Some(Decimal::new(10, 0)),
            Some(Decimal::new(5, 0)),
            Some(Decimal::new(15, 1)),
        )
    );
    assert_eq!(
        (
            fields.base.tpm_limit,
            fields.base.rpm_limit,
            fields.base.tpd_limit,
        ),
        (Some(10), Some(20), Some(30))
    );
    assert!(fields.base.blocked);
    assert_eq!(fields.base.models, ["gpt-4".to_owned()]);
    assert_eq!(fields.object_permission_id.as_deref(), Some("perm"));
    let metadata = fields.base.metadata.as_ref().expect("team metadata");
    assert!(metadata.contains_key("k"));
    let model_max_budget = fields
        .model_max_budget
        .0
        .as_ref()
        .expect("model max budget");
    assert!(model_max_budget.contains_key("gpt-4"));
    assert_eq!(fields.model_spend, Nullable(Some(BTreeMap::new())));
    assert_eq!(fields.allow_team_guardrail_config, Nullable(Some(false)));
    assert_eq!(
        (
            fields.base.organization_id.is_none(),
            fields.base.team_alias.is_none(),
            cached.last_refreshed_at.is_none(),
        ),
        (true, true, true)
    );
}

#[test]
fn reconstructed_team_rejects_missing_id_and_non_string_model() {
    assert!(matches!(
        team_obj_from_token(&key(r#"{"token":"k"}"#)),
        Err(TeamFromTokenError::MissingTeamId)
    ));
    assert!(matches!(
        team_obj_from_token(&key(r#"{"token":"k","team_id":"t1","team_models":[1]}"#)),
        Err(TeamFromTokenError::NonStringModel { index: 0 })
    ));
}

#[rstest]
#[case(TeamLookupError::TeamNotFound {
    team_id: "litellm-dashboard".to_owned(),
})]
#[case(TeamLookupError::NoDatabase)]
#[case(TeamLookupError::NotInCache {
    team_id: "litellm-dashboard".to_owned(),
})]
fn dashboard_vouches_for_missing_team(#[case] error: TeamLookupError) {
    assert_eq!(UI_TEAM_ID, "litellm-dashboard");
    let token = key(r#"{"token":"k","team_id":"litellm-dashboard"}"#);
    let outage = AllowRequestsOnOutage::Unsupported;
    assert!(token_can_vouch_for_team(&token, &error, &outage).unwrap());
}

#[test]
fn missing_team_never_vouches() {
    let token = key(r#"{"token":"k","team_id":"t1","team_models":["gpt-4"]}"#);
    let not_found = TeamLookupError::TeamNotFound {
        team_id: "t1".to_owned(),
    };
    let deleted_team =
        token_can_vouch_for_team(&token, &not_found, &AllowRequestsOnOutage::Boolean(true));
    assert!(!deleted_team.unwrap());
    let bare = key(r#"{"token":"k","team_id":"t1"}"#);
    let unavailable = TeamLookupError::TeamUnavailable {
        team_id: "t1".to_owned(),
        cause: Box::new(TeamFailure::Throttled),
    };
    let outage_denied =
        token_can_vouch_for_team(&bare, &unavailable, &AllowRequestsOnOutage::Boolean(false));
    assert!(!outage_denied.unwrap());
    let without_team = key(r#"{"token":"k"}"#);
    let no_team_id = token_can_vouch_for_team(
        &without_team,
        &TeamLookupError::NoDatabase,
        &AllowRequestsOnOutage::Boolean(true),
    );
    assert!(!no_team_id.unwrap());
}

#[rstest]
#[case(TeamLookupError::TeamUnavailable {
    team_id: "t1".to_owned(),
    cause: Box::new(TeamFailure::Throttled),
})]
#[case(TeamLookupError::NoDatabase)]
#[case(TeamLookupError::NotInCache {
    team_id: "t1".to_owned(),
})]
#[case(TeamLookupError::AliasNotFound {
    team_alias: "a".to_owned(),
})]
fn nonempty_team_models_bypass_outage_setting(#[case] error: TeamLookupError) {
    let token = key(r#"{"token":"k","team_id":"t1","team_models":["gpt-4"]}"#);
    let vouched = token_can_vouch_for_team(&token, &error, &AllowRequestsOnOutage::Unsupported);
    assert!(vouched.unwrap());
}

#[test]
fn empty_models_consult_outage_setting() {
    let token = key(r#"{"token":"k","team_id":"t1"}"#);
    let unavailable = TeamLookupError::TeamUnavailable {
        team_id: "t1".to_owned(),
        cause: Box::new(TeamFailure::Throttled),
    };
    let allowed =
        token_can_vouch_for_team(&token, &unavailable, &AllowRequestsOnOutage::Boolean(true));
    assert!(allowed.unwrap());
    let denied =
        token_can_vouch_for_team(&token, &unavailable, &AllowRequestsOnOutage::Boolean(false));
    assert!(!denied.unwrap());
    let text = AllowRequestsOnOutage::Text(" TRUE ".to_owned());
    assert!(token_can_vouch_for_team(&token, &unavailable, &text).unwrap());
    assert!(matches!(
        token_can_vouch_for_team(&token, &unavailable, &AllowRequestsOnOutage::Unsupported),
        Err(PolicyError::UnsupportedValue)
    ));
}

#[tokio::test]
async fn org_id_inherits_first() {
    let world = org_world();
    let lookup = org_lookup_without_database(&world);
    let mut token = key(r#"{"token":"k","team_id":"t1"}"#);
    let team_row = team(r#"{"team_id":"t1","organization_id":"o1"}"#);
    inherit(&mut token, &lookup, Some(&team_row)).await;
    assert_eq!(token.view.token.org_id.as_deref(), Some("o1"));
    assert!(token.view.organization_alias.is_none());

    let mut existing = key(r#"{"token":"k","team_id":"t1","org_id":"o2"}"#);
    inherit(&mut existing, &lookup, Some(&team_row)).await;
    assert_eq!(existing.view.token.org_id.as_deref(), Some("o2"));
}

#[rstest]
#[case(r#"{"token":"k","organization_alias":""}"#)]
#[case(r#"{"token":"k","organization_max_budget":0}"#)]
#[case(r#"{"token":"k","organization_tpm_limit":0}"#)]
#[case(r#"{"token":"k","organization_rpm_limit":0}"#)]
#[case(r#"{"token":"k","organization_metadata":{}}"#)]
#[tokio::test]
async fn any_populated_org_field_skips(#[case] fields: &str) {
    let world = org_world();
    world.seed(org_row(
        r#"{"organization_id":"o1","organization_alias":"Acme","budget_id":"b1","created_by":"admin","updated_by":"admin"}"#,
    ));
    let lookup = org_lookup(&world);
    let team_row = team(r#"{"team_id":"t1","organization_id":"o1"}"#);
    let mut token = key(fields);
    inherit(&mut token, &lookup, Some(&team_row)).await;
    assert_eq!(token.view.token.org_id.as_deref(), Some("o1"), "{fields}");
    assert!(token.view.organization_alias.is_none(), "{fields}");
    assert_eq!(world.reads(), 0);
}

#[tokio::test]
async fn org_lookup_missing_db_or_id() {
    let world = org_world();
    world.seed(org_row(
        r#"{"organization_id":"o1","organization_alias":"Acme","budget_id":"b1","created_by":"admin","updated_by":"admin"}"#,
    ));
    let mut without_database = key(r#"{"token":"k","org_id":"o1"}"#);
    inherit(
        &mut without_database,
        &org_lookup_without_database(&world),
        None,
    )
    .await;
    assert!(without_database.view.organization_alias.is_none());
    assert_eq!(world.reads(), 0);

    let mut without_id = key(r#"{"token":"k"}"#);
    inherit(&mut without_id, &org_lookup(&world), None).await;
    assert!(without_id.view.token.org_id.is_none());
    assert_eq!(world.reads(), 0);
}

#[tokio::test]
async fn org_missing_row_noop() {
    let world = org_world();
    let mut token = key(r#"{"token":"k","org_id":"o1"}"#);
    inherit(&mut token, &org_lookup(&world), None).await;
    assert_eq!(token.view.token.org_id.as_deref(), Some("o1"));
    assert!(token.view.organization_alias.is_none());
    assert!(token.view.organization_metadata.is_none());
    assert!(token.view.organization_max_budget.is_none());
    assert_eq!(world.reads(), 1);
}

#[tokio::test]
async fn org_without_budget_copies_identity() {
    let world = org_world();
    world.seed(org_row(
        r#"{"organization_id":"o1","organization_alias":"Acme","budget_id":"b1","created_by":"admin","updated_by":"admin","metadata":{"tier":"gold"}}"#,
    ));
    let mut token = key(r#"{"token":"k","org_id":"o1"}"#);
    inherit(&mut token, &org_lookup(&world), None).await;
    assert_eq!(token.view.organization_alias.as_deref(), Some("Acme"));
    let metadata = token
        .view
        .organization_metadata
        .as_ref()
        .expect("org metadata");
    assert!(metadata.0.contains_key("tier"));
    assert!(token.view.organization_max_budget.is_none());
    assert!(token.view.organization_tpm_limit.is_none());
    assert!(token.view.organization_rpm_limit.is_none());
}

#[tokio::test]
async fn org_budget_fields_copy() {
    let world = org_world();
    world.seed(org_row(
        r#"{"organization_id":"o1","organization_alias":"Acme","budget_id":"b1","created_by":"admin","updated_by":"admin","litellm_budget_table":{"budget_id":"b1","max_budget":7,"tpm_limit":5,"rpm_limit":6}}"#,
    ));
    let mut token = key(r#"{"token":"k","team_id":"t1"}"#);
    let team_row = team(r#"{"team_id":"t1","organization_id":"o1"}"#);
    inherit(&mut token, &org_lookup(&world), Some(&team_row)).await;
    assert_eq!(token.view.token.org_id.as_deref(), Some("o1"));
    assert_eq!(token.view.organization_alias.as_deref(), Some("Acme"));
    assert_eq!(token.view.organization_max_budget, Some(Decimal::new(7, 0)));
    assert_eq!(token.view.organization_tpm_limit, Some(5));
    assert_eq!(token.view.organization_rpm_limit, Some(6));
}
