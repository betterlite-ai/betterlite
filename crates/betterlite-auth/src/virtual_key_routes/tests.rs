#![cfg(test)]

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;
use betterlite_error::proxy_exception::ProxyException;
use rstest::rstest;

use crate::pass_through_registry::{
    PassThroughMetadata, PassThroughRegistry, PassThroughRouteType,
};
use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use crate::route_match::groups::get_actual_routes;

use super::caller::{
    AllowedRoutes, RouteCaller, check_passthrough_route_access, jwt_team_allowed_routes,
    jwt_team_grant_pass_through, password_reset_required,
};
use super::groups::group_routes;
use super::pass_through::require_pass_through_access;
use super::route_checks::{CalledRoute, check_virtual_key_route};

const ALL_METHODS: [&str; 5] = ["GET", "POST", "PUT", "DELETE", "PATCH"];

fn strings(values: &[&str]) -> JsonData {
    let items = values
        .iter()
        .map(|value| JsonData::String((*value).to_owned()))
        .collect();
    JsonData::Array(items)
}

fn object(entries: &[(&str, JsonData)]) -> JsonData {
    let fields = entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect();
    JsonData::Object(fields)
}

fn payload(entries: &[(&str, JsonData)]) -> Payload {
    let fields = entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect();
    Payload(fields)
}

fn key_of(entries: Vec<(&str, JsonData)>) -> UserAPIKeyAuth {
    let fields = entries
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect();
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap()
}

fn jwt_key(extra: Vec<(&str, JsonData)>) -> UserAPIKeyAuth {
    let mut entries = vec![(
        "jwt_claims",
        object(&[("sub", JsonData::String("user-a".to_owned()))]),
    )];
    entries.extend(extra);
    key_of(entries)
}

const fn called<'route>(route: &'route str, verb: &'route str) -> CalledRoute<'route> {
    let method = Some(verb);
    CalledRoute { route, method }
}

fn registration(
    endpoint_id: &str,
    route_type: PassThroughRouteType,
    path: &str,
    methods: &[&str],
    auth: Option<bool>,
) -> (String, PassThroughMetadata) {
    let label = match route_type {
        PassThroughRouteType::Exact => "exact",
        PassThroughRouteType::Subpath => "subpath",
    };
    let stored = methods.iter().map(|method| (*method).to_owned()).collect();
    let route_key = format!("{endpoint_id}:{label}:{path}:{}", methods.join(","));
    let metadata = PassThroughMetadata {
        endpoint_id: endpoint_id.to_owned(),
        route_type,
        path: path.to_owned(),
        methods: stored,
        auth,
    };
    (route_key, metadata)
}

fn registry_of(
    entries: &[(&str, PassThroughRouteType, &str, Option<bool>)],
) -> PassThroughRegistry {
    let registrations = entries
        .iter()
        .map(|(endpoint_id, route_type, path, auth)| {
            registration(endpoint_id, *route_type, path, &ALL_METHODS, *auth)
        })
        .collect();
    PassThroughRegistry {
        root_path: String::new(),
        registrations,
    }
}

fn count_route(routes: &[String], path: &str) -> usize {
    routes.iter().filter(|route| route.as_str() == path).count()
}

fn assert_forbidden(exception: &ProxyException) -> &str {
    assert_eq!(exception.code, "403");
    assert_eq!(exception.error_type, "auth_error");
    assert_eq!(exception.param.as_deref(), Some("None"));
    &exception.message
}

#[test]
fn mit_enterprise_import_absent() {
    let auth = key_of(vec![("allowed_routes", strings(&["/v1/models"]))]);
    let caller = RouteCaller::of_key(&auth);
    let registry = registry_of(&[]);
    let exception =
        *check_virtual_key_route(called("/v1/chat/completions", "POST"), &caller, &registry)
            .unwrap_err();
    // the MIT fallback owns the denial: no enterprise check turns this into another error shape
    assert!(
        assert_forbidden(&exception).starts_with("Virtual key is not allowed to call this route.")
    );
}

#[test]
fn routes_none_allows() {
    let auth = key_of(vec![
        ("allowed_routes", JsonData::Null),
        (
            "metadata",
            object(&[("password_reset_required", JsonData::Bool(true))]),
        ),
    ]);
    let caller = RouteCaller::of_key(&auth);
    assert_eq!(caller.allowed_routes, AllowedRoutes::Unset);
    let registry = registry_of(&[(
        "pass-a",
        PassThroughRouteType::Exact,
        "/team-pass",
        Some(true),
    )]);
    check_virtual_key_route(called("/team-pass", "POST"), &caller, &registry).unwrap();
}

#[test]
fn routes_nonlist_allows() {
    let caller = RouteCaller {
        allowed_routes: AllowedRoutes::NotAList,
        ..RouteCaller::absent()
    };
    let registry = registry_of(&[(
        "pass-a",
        PassThroughRouteType::Exact,
        "/team-pass",
        Some(true),
    )]);
    check_virtual_key_route(called("/team-pass", "POST"), &caller, &registry).unwrap();
}

#[test]
fn routes_empty_allows() {
    let auth = key_of(vec![
        ("allowed_routes", strings(&[])),
        (
            "metadata",
            object(&[("password_reset_required", JsonData::Bool(true))]),
        ),
    ]);
    let caller = RouteCaller::of_key(&auth);
    assert_eq!(caller.allowed_routes, AllowedRoutes::Listed(&[]));
    let registry = registry_of(&[(
        "pass-a",
        PassThroughRouteType::Exact,
        "/team-pass",
        Some(true),
    )]);
    check_virtual_key_route(called("/team-pass", "POST"), &caller, &registry).unwrap();
}

#[test]
fn explicit_route_grant() {
    let auth = key_of(vec![("allowed_routes", strings(&["/v1/models"]))]);
    let caller = RouteCaller::of_key(&auth);
    let registry = registry_of(&[]);
    check_virtual_key_route(called("/v1/models", "GET"), &caller, &registry).unwrap();
    check_virtual_key_route(called("/v1/models/model-a", "GET"), &caller, &registry).unwrap();
    // the prefix match needs the path separator
    check_virtual_key_route(called("/v1/modelsx", "GET"), &caller, &registry).unwrap_err();
}

#[test]
fn non_string_route_entries_are_ignored() {
    let auth = key_of(vec![(
        "allowed_routes",
        JsonData::Array(vec![
            JsonData::Number(serde_json::Number::from(7_i32)),
            JsonData::String("/v1/models".to_owned()),
        ]),
    )]);
    let caller = RouteCaller::of_key(&auth);
    let registry = registry_of(&[]);
    // the number never matches and never raises, while the string beside it grants
    check_virtual_key_route(called("/v1/models", "GET"), &caller, &registry).unwrap();
    let numbers_only = key_of(vec![(
        "allowed_routes",
        JsonData::Array(vec![JsonData::Number(serde_json::Number::from(7_i32))]),
    )]);
    let numbers_caller = RouteCaller::of_key(&numbers_only);
    let exception =
        *check_virtual_key_route(called("/v1/models", "GET"), &numbers_caller, &registry)
            .unwrap_err();
    assert!(assert_forbidden(&exception).contains("Only allowed to call routes: []"));
}

#[test]
fn named_group_grant() {
    assert_ne!(
        get_actual_routes(&["openai_routes"]),
        ["openai_routes".to_owned()]
    );
    assert_eq!(
        get_actual_routes(&["/v1/chat/completions"]),
        ["/v1/chat/completions".to_owned()]
    );
    let auth = key_of(vec![("allowed_routes", strings(&["openai_routes"]))]);
    let caller = RouteCaller::of_key(&auth);
    let registry = registry_of(&[]);
    check_virtual_key_route(called("/v1/chat/completions", "POST"), &caller, &registry).unwrap();
    check_virtual_key_route(called("/v1/embeddings", "POST"), &caller, &registry).unwrap();
}

#[test]
fn wildcard_before_denial() {
    let auth = key_of(vec![(
        "allowed_routes",
        strings(&["openai_routes", "/team-pass/*"]),
    )]);
    let caller = RouteCaller::of_key(&auth);
    let registry = registry_of(&[(
        "pass-a",
        PassThroughRouteType::Subpath,
        "/team-pass",
        Some(true),
    )]);
    // the group match remembers the protected pass-through denial, and the wildcard still wins
    check_virtual_key_route(called("/team-pass/x", "POST"), &caller, &registry).unwrap();
}

#[test]
fn team_pass_through_grant() {
    let auth = key_of(vec![
        ("allowed_routes", strings(&["llm_api_routes"])),
        (
            "team_metadata",
            object(&[("allowed_passthrough_routes", strings(&["/team-pass"]))]),
        ),
    ]);
    let caller = RouteCaller::of_key(&auth);
    let registry = registry_of(&[(
        "pass-a",
        PassThroughRouteType::Exact,
        "/team-pass",
        Some(true),
    )]);
    check_virtual_key_route(called("/team-pass", "POST"), &caller, &registry).unwrap();
}

#[test]
fn empty_key_uses_team() {
    let auth = key_of(vec![
        ("allowed_routes", strings(&["llm_api_routes"])),
        (
            "metadata",
            object(&[("allowed_passthrough_routes", strings(&[]))]),
        ),
        (
            "team_metadata",
            object(&[("allowed_passthrough_routes", strings(&["/team-pass"]))]),
        ),
    ]);
    let caller = RouteCaller::of_key(&auth);
    let registry = registry_of(&[(
        "pass-a",
        PassThroughRouteType::Exact,
        "/team-pass",
        Some(true),
    )]);
    check_virtual_key_route(called("/team-pass", "POST"), &caller, &registry).unwrap();
}

#[test]
fn nonempty_key_overrides_team() {
    let auth = key_of(vec![
        ("allowed_routes", strings(&["llm_api_routes"])),
        (
            "metadata",
            object(&[("allowed_passthrough_routes", strings(&["/other"]))]),
        ),
        (
            "team_metadata",
            object(&[("allowed_passthrough_routes", strings(&["/team-pass"]))]),
        ),
    ]);
    let caller = RouteCaller::of_key(&auth);
    let registry = registry_of(&[(
        "pass-a",
        PassThroughRouteType::Exact,
        "/team-pass",
        Some(true),
    )]);
    let exception =
        *check_virtual_key_route(called("/team-pass", "POST"), &caller, &registry).unwrap_err();
    assert!(assert_forbidden(&exception).contains("passthrough route /team-pass"));
}

#[test]
fn missing_metadata_grant() {
    let auth = key_of(vec![
        ("allowed_routes", strings(&["llm_api_routes"])),
        ("metadata", object(&[])),
    ]);
    let caller = RouteCaller::of_key(&auth);
    let registry = registry_of(&[(
        "pass-a",
        PassThroughRouteType::Exact,
        "/team-pass",
        Some(true),
    )]);
    let exception =
        *check_virtual_key_route(called("/team-pass", "POST"), &caller, &registry).unwrap_err();
    assert!(assert_forbidden(&exception).contains("Configure `allowed_passthrough_routes`"));
}

#[test]
fn null_metadata_grants() {
    let auth = key_of(vec![
        ("allowed_routes", strings(&["llm_api_routes"])),
        (
            "metadata",
            object(&[("allowed_passthrough_routes", JsonData::Null)]),
        ),
        (
            "team_metadata",
            object(&[("allowed_passthrough_routes", JsonData::Null)]),
        ),
    ]);
    let caller = RouteCaller::of_key(&auth);
    let registry = registry_of(&[(
        "pass-a",
        PassThroughRouteType::Exact,
        "/team-pass",
        Some(true),
    )]);
    let exception =
        *check_virtual_key_route(called("/team-pass", "POST"), &caller, &registry).unwrap_err();
    assert!(assert_forbidden(&exception).contains("Configure `allowed_passthrough_routes`"));
}

#[test]
fn jwt_team_only() {
    let auth = jwt_key(vec![("team_id", JsonData::String("team-a".to_owned()))]);
    let caller = RouteCaller::of_key(&auth);
    let settings = [
        "openai_routes".to_owned(),
        "info_routes".to_owned(),
        "mcp_routes".to_owned(),
        "/v1/messages".to_owned(),
        "/v1/messages/count_tokens".to_owned(),
    ];
    let routes = jwt_team_allowed_routes(&caller, &settings);
    assert_eq!(routes, settings.as_slice());
    let plain = key_of(vec![("team_id", JsonData::String("team-a".to_owned()))]);
    let plain_caller = RouteCaller::of_key(&plain);
    assert!(jwt_team_allowed_routes(&plain_caller, &settings).is_empty());
    assert!(jwt_team_allowed_routes(&RouteCaller::absent(), &settings).is_empty());
    require_pass_through_access("/team-pass", &caller, routes).unwrap_err();
}

#[test]
fn jwt_mapped_key_excluded() {
    let auth = jwt_key(vec![
        ("team_id", JsonData::String("team-a".to_owned())),
        ("token", JsonData::String("sk-key-a".to_owned())),
    ]);
    let caller = RouteCaller::of_key(&auth);
    let settings = vec!["/team-pass/*".to_owned()];
    assert!(jwt_team_allowed_routes(&caller, &settings).is_empty());
    let exception = *require_pass_through_access("/team-pass/x", &caller, &[]).unwrap_err();
    assert!(assert_forbidden(&exception).contains("passthrough route /team-pass/x"));
}

#[test]
fn jwt_blanket_denied() {
    let auth = jwt_key(vec![("team_id", JsonData::String("team-a".to_owned()))]);
    let caller = RouteCaller::of_key(&auth);
    let settings = vec!["*".to_owned(), "/*".to_owned()];
    assert!(!jwt_team_grant_pass_through("/team-pass/x", &settings));
    require_pass_through_access("/team-pass/x", &caller, &settings).unwrap_err();
}

#[test]
fn jwt_group_denied() {
    let auth = jwt_key(vec![("team_id", JsonData::String("team-a".to_owned()))]);
    let caller = RouteCaller::of_key(&auth);
    let settings = vec!["openai_routes".to_owned()];
    // a named group is never expanded into the paths it names
    assert!(!jwt_team_grant_pass_through(
        "/v1/chat/completions",
        &settings
    ));
    require_pass_through_access("/v1/chat/completions", &caller, &settings).unwrap_err();
}

#[test]
fn jwt_explicit_prefix() {
    let auth = jwt_key(vec![("team_id", JsonData::String("team-a".to_owned()))]);
    let caller = RouteCaller::of_key(&auth);
    let settings = vec!["/team-pass/*".to_owned()];
    require_pass_through_access("/team-pass/x", &caller, &settings).unwrap();
    require_pass_through_access("/team-pass", &caller, &settings).unwrap_err();
    require_pass_through_access("/other", &caller, &settings).unwrap_err();
}

#[test]
fn jwt_key_metadata_grant_satisfies_require_auth_pass_through() {
    let auth = jwt_key(vec![
        ("team_id", JsonData::String("team-a".to_owned())),
        (
            "metadata",
            object(&[("allowed_passthrough_routes", strings(&["/team-pass"]))]),
        ),
    ]);
    let caller = RouteCaller::of_key(&auth);
    // the metadata grant answers before the explicit JWT paths, which are empty here
    require_pass_through_access("/team-pass", &caller, &[]).unwrap();
    require_pass_through_access("/other", &caller, &[]).unwrap_err();
}

#[rstest]
#[case("/v1/mcp/server")]
#[case("/v1/mcp/server/server-a")]
fn mcp_discovery_get(#[case] route: &str) {
    let auth = key_of(vec![("allowed_routes", strings(&["llm_api_routes"]))]);
    let caller = RouteCaller::of_key(&auth);
    let registry = registry_of(&[]);
    check_virtual_key_route(called(route, "GET"), &caller, &registry).unwrap();
}

#[rstest]
#[case("/v1/mcp/server")]
#[case("/v1/mcp/server/server-a")]
fn mcp_discovery_write_denied(#[case] route: &str) {
    let auth = key_of(vec![("allowed_routes", strings(&["llm_api_routes"]))]);
    let caller = RouteCaller::of_key(&auth);
    let registry = registry_of(&[]);
    let exception =
        *check_virtual_key_route(called(route, "POST"), &caller, &registry).unwrap_err();
    assert!(
        assert_forbidden(&exception).starts_with("Virtual key is not allowed to call this route.")
    );
}

#[test]
fn auto_router_get() {
    let auth = key_of(vec![("allowed_routes", strings(&["llm_api_routes"]))]);
    let caller = RouteCaller::of_key(&auth);
    let registry = registry_of(&[]);
    check_virtual_key_route(called("/auto_router/session", "GET"), &caller, &registry).unwrap();
}

#[rstest]
#[case("POST")]
#[case("DELETE")]
fn auto_router_write_denied(#[case] method: &str) {
    let auth = key_of(vec![("allowed_routes", strings(&["llm_api_routes"]))]);
    let caller = RouteCaller::of_key(&auth);
    let registry = registry_of(&[]);
    check_virtual_key_route(called("/auto_router/session", method), &caller, &registry)
        .unwrap_err();
}

#[test]
fn agent_compatibility_grant() {
    let auth = key_of(vec![("allowed_routes", strings(&["llm_api_routes"]))]);
    let caller = RouteCaller::of_key(&auth);
    let registry = registry_of(&[]);
    check_virtual_key_route(called("/v1/agents", "POST"), &caller, &registry).unwrap();
    check_virtual_key_route(
        called("/v1/agents/agent-a/kill_switch", "POST"),
        &caller,
        &registry,
    )
    .unwrap();
}

#[test]
fn password_reset_denied() {
    let auth = key_of(vec![
        ("allowed_routes", strings(&["/v1/models"])),
        (
            "metadata",
            object(&[("password_reset_required", JsonData::Bool(true))]),
        ),
    ]);
    let caller = RouteCaller::of_key(&auth);
    let registry = registry_of(&[]);
    let exception =
        *check_virtual_key_route(called("/v1/chat/completions", "POST"), &caller, &registry)
            .unwrap_err();
    let message = assert_forbidden(&exception);
    assert!(message.contains("password must be changed"));
    assert!(message.contains("/user/password/change"));
}

#[test]
fn only_a_true_reset_flag_denies() {
    let auth = key_of(vec![
        ("allowed_routes", strings(&["/v1/models"])),
        (
            "metadata",
            object(&[(
                "password_reset_required",
                JsonData::Number(serde_json::Number::from(1_i32)),
            )]),
        ),
    ]);
    let caller = RouteCaller::of_key(&auth);
    let registry = registry_of(&[]);
    // LiteLLM tests `is True`, so a truthy number leaves the ordinary denial in place
    let exception =
        *check_virtual_key_route(called("/v1/chat/completions", "POST"), &caller, &registry)
            .unwrap_err();
    assert!(
        assert_forbidden(&exception).starts_with("Virtual key is not allowed to call this route.")
    );
}

#[test]
fn password_reset_allowed_route() {
    let auth = key_of(vec![
        ("allowed_routes", strings(&["/v1/models"])),
        (
            "metadata",
            object(&[("password_reset_required", JsonData::Bool(true))]),
        ),
    ]);
    let caller = RouteCaller::of_key(&auth);
    let registry = registry_of(&[]);
    check_virtual_key_route(called("/v1/models", "GET"), &caller, &registry).unwrap();
}

#[test]
fn virtual_route_denied() {
    let auth = key_of(vec![("allowed_routes", strings(&["/v1/models"]))]);
    let caller = RouteCaller::of_key(&auth);
    let registry = registry_of(&[]);
    let exception =
        *check_virtual_key_route(called("/v1/chat/completions", "POST"), &caller, &registry)
            .unwrap_err();
    assert_eq!(
        assert_forbidden(&exception),
        "Virtual key is not allowed to call this route. Only allowed to call routes: ['/v1/models']. Tried to call route: /v1/chat/completions"
    );
}

#[test]
fn registered_protected_route() {
    let auth = key_of(vec![("allowed_routes", strings(&["openai_routes"]))]);
    let caller = RouteCaller::of_key(&auth);
    let registry = registry_of(&[(
        "pass-a",
        PassThroughRouteType::Exact,
        "/team-pass",
        Some(true),
    )]);
    let exception =
        *check_virtual_key_route(called("/team-pass", "POST"), &caller, &registry).unwrap_err();
    assert!(
        assert_forbidden(&exception)
            .starts_with("Key/team not allowed to access passthrough route")
    );
}

#[test]
fn absent_auth_unprotected() {
    let auth = key_of(vec![("allowed_routes", strings(&["openai_routes"]))]);
    let caller = RouteCaller::of_key(&auth);
    let registry = registry_of(&[("pass-a", PassThroughRouteType::Exact, "/team-pass", None)]);
    // only an auth=true registration joins openai_routes, so an absent flag grants nothing there
    let exception =
        *check_virtual_key_route(called("/team-pass", "POST"), &caller, &registry).unwrap_err();
    assert!(
        assert_forbidden(&exception).starts_with("Virtual key is not allowed to call this route.")
    );
    // llm_api_routes reaches the registration itself, where an absent flag leaves it open
    let llm_auth = key_of(vec![("allowed_routes", strings(&["llm_api_routes"]))]);
    let llm_caller = RouteCaller::of_key(&llm_auth);
    check_virtual_key_route(called("/team-pass", "POST"), &llm_caller, &registry).unwrap();
}

#[test]
fn passthrough_denial_precedes_reset() {
    let auth = key_of(vec![
        ("allowed_routes", strings(&["openai_routes"])),
        (
            "metadata",
            object(&[("password_reset_required", JsonData::Bool(true))]),
        ),
    ]);
    let caller = RouteCaller::of_key(&auth);
    let registry = registry_of(&[(
        "pass-a",
        PassThroughRouteType::Exact,
        "/team-pass",
        Some(true),
    )]);
    let exception =
        *check_virtual_key_route(called("/team-pass", "POST"), &caller, &registry).unwrap_err();
    assert!(exception.message.contains("passthrough route /team-pass"));
    assert!(!exception.message.contains("password"));
}

#[test]
fn absent_caller_holds_no_key_facts() {
    let caller = RouteCaller::absent();
    assert_eq!(caller.allowed_routes, AllowedRoutes::Unset);
    assert!(caller.metadata.is_none());
    assert!(caller.team_metadata.is_none());
    assert!(!caller.jwt_claims);
    assert!(!caller.has_token);
    assert!(caller.team_id.is_none());
    assert!(!check_passthrough_route_access("/team-pass", &caller));
    assert!(!password_reset_required(&caller));
}

#[test]
fn openai_routes_group_includes_protected_registrations() {
    let registry = registry_of(&[
        (
            "pass-a",
            PassThroughRouteType::Exact,
            "/exact-pass",
            Some(true),
        ),
        (
            "pass-b",
            PassThroughRouteType::Subpath,
            "/sub-pass",
            Some(true),
        ),
        (
            "pass-c",
            PassThroughRouteType::Exact,
            "/open-pass",
            Some(false),
        ),
    ]);
    let routes = group_routes("openai_routes", &registry);
    assert!(routes.contains(&"/exact-pass".to_owned()));
    assert!(routes.contains(&"/sub-pass".to_owned()));
    assert!(routes.contains(&"/sub-pass/*".to_owned()));
    assert!(!routes.contains(&"/exact-pass/*".to_owned()));
    assert!(!routes.contains(&"/open-pass".to_owned()));
    assert!(routes.contains(&"/v1/chat/completions".to_owned()));
    // the other groups never see runtime registrations
    assert!(!group_routes("llm_api_routes", &registry).contains(&"/exact-pass".to_owned()));
}

#[test]
fn method_filtered_registration_is_unprotected() {
    let auth = key_of(vec![("allowed_routes", strings(&["openai_routes"]))]);
    let caller = RouteCaller::of_key(&auth);
    let registry = PassThroughRegistry {
        root_path: String::new(),
        registrations: vec![registration(
            "pass-a",
            PassThroughRouteType::Exact,
            "/team-pass",
            &["POST"],
            Some(true),
        )],
    };
    check_virtual_key_route(called("/team-pass", "GET"), &caller, &registry).unwrap();
    let exception =
        *check_virtual_key_route(called("/team-pass", "POST"), &caller, &registry).unwrap_err();
    assert!(exception.message.contains("passthrough route /team-pass"));
}

#[rstest]
#[case(JsonData::String(String::new()))]
#[case(object(&[]))]
#[case(JsonData::Bool(false))]
#[case(JsonData::Number(serde_json::Number::from(0_i32)))]
#[case(JsonData::Null)]
fn falsy_grants_fall_back_to_the_team_list(#[case] value: JsonData) {
    let team = payload(&[("allowed_passthrough_routes", strings(&["/team-pass"]))]);
    let key = payload(&[("allowed_passthrough_routes", value)]);
    let caller = RouteCaller {
        metadata: Some(&key),
        team_metadata: Some(&team),
        ..RouteCaller::absent()
    };
    assert!(check_passthrough_route_access("/team-pass", &caller));
}

#[rstest]
#[case(JsonData::Bool(true))]
#[case(JsonData::Number(serde_json::Number::from(5_i32)))]
#[case(JsonData::Number(
    serde_json::Number::from_f64(0.5).expect("a finite number")
))]
fn non_iterable_truthy_grants_deny(#[case] value: JsonData) {
    let team = payload(&[("allowed_passthrough_routes", strings(&["/team-pass"]))]);
    let key = payload(&[("allowed_passthrough_routes", value)]);
    let caller = RouteCaller {
        metadata: Some(&key),
        team_metadata: Some(&team),
        ..RouteCaller::absent()
    };
    // the truthy key value wins over the team grant and denies, rather than raising
    assert!(!check_passthrough_route_access("/team-pass", &caller));
}

#[test]
fn string_and_object_grants_iterate_like_python() {
    // a string is iterated by character, so a full path in a string grants nothing
    let text = payload(&[(
        "allowed_passthrough_routes",
        JsonData::String("/team-pass".to_owned()),
    )]);
    let text_caller = RouteCaller {
        metadata: Some(&text),
        ..RouteCaller::absent()
    };
    assert!(!check_passthrough_route_access("/team-pass", &text_caller));
    // an object is iterated by key
    let map = payload(&[(
        "allowed_passthrough_routes",
        object(&[("/team-pass", JsonData::Null)]),
    )]);
    let map_caller = RouteCaller {
        metadata: Some(&map),
        ..RouteCaller::absent()
    };
    assert!(check_passthrough_route_access("/team-pass", &map_caller));
    // non-string list entries are skipped rather than raising
    let mixed = payload(&[(
        "allowed_passthrough_routes",
        JsonData::Array(vec![
            JsonData::Number(serde_json::Number::from(7_i32)),
            JsonData::String("/team-pass".to_owned()),
        ]),
    )]);
    let mixed_caller = RouteCaller {
        metadata: Some(&mixed),
        ..RouteCaller::absent()
    };
    assert!(check_passthrough_route_access("/team-pass", &mixed_caller));
    // a one-character string is the one value that matches through the character iteration
    let single = payload(&[(
        "allowed_passthrough_routes",
        JsonData::String("x".to_owned()),
    )]);
    let single_caller = RouteCaller {
        metadata: Some(&single),
        ..RouteCaller::absent()
    };
    assert!(check_passthrough_route_access("x", &single_caller));
}

#[test]
fn denial_message_lists_every_allowed_route() {
    let auth = key_of(vec![(
        "allowed_routes",
        strings(&["/v1/models", "/v1/embeddings"]),
    )]);
    let caller = RouteCaller::of_key(&auth);
    let registry = registry_of(&[]);
    let exception =
        *check_virtual_key_route(called("/v1/chat/completions", "POST"), &caller, &registry)
            .unwrap_err();
    assert_eq!(
        assert_forbidden(&exception),
        "Virtual key is not allowed to call this route. Only allowed to call routes: ['/v1/models', '/v1/embeddings']. Tried to call route: /v1/chat/completions"
    );
}

#[test]
fn denial_message_escapes_route_text() {
    let auth = key_of(vec![(
        "allowed_routes",
        strings(&["/v1/it's", "/v1/back\\slash"]),
    )]);
    let caller = RouteCaller::of_key(&auth);
    let registry = registry_of(&[]);
    let exception =
        *check_virtual_key_route(called("/v1/chat/completions", "POST"), &caller, &registry)
            .unwrap_err();
    // the message renders the list like Python's repr, escaping quotes and backslashes
    assert!(assert_forbidden(&exception).contains(r"['/v1/it\'s', '/v1/back\\slash']"));
}

#[test]
fn only_member_names_expand_as_groups() {
    let auth = key_of(vec![("allowed_routes", strings(&["/v1/models/{model}"]))]);
    let caller = RouteCaller::of_key(&auth);
    let registry = registry_of(&[]);
    // a path-shaped entry is not a named group, so its placeholder reaches no route check
    let exception =
        *check_virtual_key_route(called("/v1/models/model-a", "GET"), &caller, &registry)
            .unwrap_err();
    assert!(assert_forbidden(&exception).contains("['/v1/models/{model}']"));
}

#[test]
fn group_entry_denies_route_outside_the_group() {
    let auth = key_of(vec![("allowed_routes", strings(&["openai_routes"]))]);
    let caller = RouteCaller::of_key(&auth);
    let registry = registry_of(&[]);
    let exception =
        *check_virtual_key_route(called("/spend/logs", "GET"), &caller, &registry).unwrap_err();
    assert!(assert_forbidden(&exception).contains("['openai_routes']"));
}

#[test]
fn openai_group_grant_covers_protected_registration() {
    let auth = key_of(vec![
        ("allowed_routes", strings(&["openai_routes"])),
        (
            "metadata",
            object(&[("allowed_passthrough_routes", strings(&["/team-pass"]))]),
        ),
    ]);
    let caller = RouteCaller::of_key(&auth);
    let registry = registry_of(&[(
        "pass-a",
        PassThroughRouteType::Exact,
        "/team-pass",
        Some(true),
    )]);
    check_virtual_key_route(called("/team-pass", "POST"), &caller, &registry).unwrap();
}

#[test]
fn llm_api_registered_open_route_granted() {
    let auth = key_of(vec![("allowed_routes", strings(&["llm_api_routes"]))]);
    let caller = RouteCaller::of_key(&auth);
    let registry = registry_of(&[(
        "pass-a",
        PassThroughRouteType::Exact,
        "/open-pass",
        Some(false),
    )]);
    check_virtual_key_route(called("/open-pass", "GET"), &caller, &registry).unwrap();
}

#[test]
fn mcp_group_skips_the_registration_auth_check() {
    let auth = key_of(vec![("allowed_routes", strings(&["mcp_routes"]))]);
    let caller = RouteCaller::of_key(&auth);
    let registry = registry_of(&[(
        "pass-a",
        PassThroughRouteType::Exact,
        "/v1/mcp/server",
        Some(true),
    )]);
    check_virtual_key_route(called("/v1/mcp/server", "GET"), &caller, &registry).unwrap();
}

#[test]
fn duplicate_registrations_appear_once() {
    let registry = registry_of(&[
        (
            "pass-a",
            PassThroughRouteType::Exact,
            "/exact-pass",
            Some(true),
        ),
        (
            "pass-d",
            PassThroughRouteType::Exact,
            "/exact-pass",
            Some(true),
        ),
        (
            "pass-b",
            PassThroughRouteType::Subpath,
            "/sub-pass",
            Some(true),
        ),
        (
            "pass-e",
            PassThroughRouteType::Subpath,
            "/sub-pass",
            Some(true),
        ),
    ]);
    let routes = group_routes("openai_routes", &registry);
    // LiteLLM appends each path only when it is not there yet, so a second registration adds nothing
    assert_eq!(count_route(&routes, "/exact-pass"), 1);
    assert_eq!(count_route(&routes, "/sub-pass/*"), 1);
    assert_eq!(count_route(&routes, "/open-pass"), 0);
}

#[test]
fn bare_wildcard_grants_every_route() {
    let auth = key_of(vec![("allowed_routes", strings(&["openai_routes", "*"]))]);
    let caller = RouteCaller::of_key(&auth);
    let registry = registry_of(&[(
        "pass-a",
        PassThroughRouteType::Exact,
        "/team-pass",
        Some(true),
    )]);
    // LiteLLM compares an empty prefix for a bare star, so it wins over the remembered pass-through denial
    check_virtual_key_route(called("/team-pass", "POST"), &caller, &registry).unwrap();
}
