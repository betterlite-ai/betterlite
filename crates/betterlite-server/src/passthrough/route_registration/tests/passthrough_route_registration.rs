use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_auth::inference_routes::OPENAI_ROUTES;
use betterlite_auth::pass_through_registry::PassThroughRouteType;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rust_decimal::Decimal;
use uuid::Uuid;

use super::super::*;
use super::support::*;

mod stale_removal;

#[test]
fn resolve_os_environ_in_headers_replaces_a_whole_value() {
    let custom = raw_headers(r#"{"Authorization":"os.environ/ANTHROPIC_API_KEY"}"#);
    let environment = environment(&[("ANTHROPIC_API_KEY", "sk-from-env")]);
    let resolved = set_env_variables_in_header(&secrets(), &environment, Some(&custom))
        .unwrap()
        .unwrap();
    assert_eq!(
        resolved.get("Authorization"),
        Some(&Json::Text("sk-from-env".to_owned()))
    );
}

#[test]
fn resolve_os_environ_in_headers_replaces_an_embedded_reference() {
    let custom = raw_headers(r#"{"Authorization":"Bearer os.environ/COHERE_API_KEY"}"#);
    let environment = environment(&[("COHERE_API_KEY", "sk-cohere")]);
    let resolved = set_env_variables_in_header(&secrets(), &environment, Some(&custom))
        .unwrap()
        .unwrap();
    assert_eq!(
        resolved.get("Authorization"),
        Some(&Json::Text("Bearer sk-cohere".to_owned()))
    );
}

#[test]
fn resolve_os_environ_in_headers_keeps_a_missing_variable() {
    let custom = raw_headers(r#"{"Authorization":"os.environ/NOT_SET"}"#);
    let resolved =
        set_env_variables_in_header(&secrets(), &BTreeMap::new(), Some(&custom)).unwrap();
    assert_eq!(resolved, Some(custom));
}

#[test]
fn resolve_os_environ_in_headers_keeps_a_boolean_environment_value() {
    let custom = raw_headers(r#"{"X-Feature":"os.environ/FEATURE_FLAG"}"#);
    let environment = environment(&[("FEATURE_FLAG", "true")]);
    let resolved = set_env_variables_in_header(&secrets(), &environment, Some(&custom))
        .unwrap()
        .unwrap();
    assert_eq!(resolved, custom);
}

#[test]
fn resolve_os_environ_in_headers_keeps_non_text_values() {
    let custom = raw_headers(
        r#"{"X-Count":3,"X-Flag":true,"X-List":[1,2],"X-Object":{"a":1},"X-Null":null}"#,
    );
    let resolved =
        set_env_variables_in_header(&secrets(), &BTreeMap::new(), Some(&custom)).unwrap();
    assert_eq!(resolved, Some(custom));
}

#[test]
fn resolve_os_environ_in_headers_returns_none_without_headers() {
    let resolved = set_env_variables_in_header(&secrets(), &BTreeMap::new(), None).unwrap();
    assert_eq!(resolved, None);
}

#[test]
fn resolve_os_environ_in_headers_returns_an_empty_map_for_empty_headers() {
    let resolved =
        set_env_variables_in_header(&secrets(), &BTreeMap::new(), Some(&JsonObject::new()))
            .unwrap();
    assert_eq!(resolved, Some(JsonObject::new()));
}

#[test]
fn resolve_os_environ_in_headers_takes_the_reference_to_the_end_of_the_value() {
    let custom = raw_headers(r#"{"X-Token":"prefix os.environ/NAME suffix"}"#);
    let environment = environment(&[("NAME suffix", "TAIL")]);
    let resolved = set_env_variables_in_header(&secrets(), &environment, Some(&custom))
        .unwrap()
        .unwrap();
    assert_eq!(
        resolved.get("X-Token"),
        Some(&Json::Text("prefix TAIL".to_owned()))
    );
}

#[test]
fn resolve_os_environ_in_headers_surfaces_a_lookup_error() {
    let custom = raw_headers(r#"{"Authorization":"os.environ/oidc/env/MISSING_TOKEN"}"#);
    let error =
        set_env_variables_in_header(&secrets(), &BTreeMap::new(), Some(&custom)).unwrap_err();
    assert!(matches!(error, RouteRegistrationError::Secrets(_)));
}

#[test]
fn resolve_os_environ_in_headers_reaches_the_registered_entry() {
    let routes = shared();
    let endpoint = parse_endpoint(
        r#"{"id":"e1","path":"/custom","target":"https://upstream","headers":{"Authorization":"Bearer os.environ/UP_KEY"}}"#,
    );
    register_with(&routes, &[endpoint], None, &[("UP_KEY", "sk-up")]);
    let state = lock_state(&routes);
    let stored = state
        .endpoints
        .get("e1:exact:/custom:DELETE,GET,PATCH,POST,PUT")
        .unwrap();
    assert_eq!(
        stored.custom_headers,
        Some(raw_headers(r#"{"Authorization":"Bearer sk-up"}"#))
    );
}

#[test]
fn langfuse_keys_become_one_basic_authorization_header() {
    let custom = raw_headers(r#"{"LANGFUSE_PUBLIC_KEY":"pk-lf","LANGFUSE_SECRET_KEY":"sk-lf"}"#);
    let resolved =
        set_env_variables_in_header(&secrets(), &BTreeMap::new(), Some(&custom)).unwrap();
    assert_eq!(
        resolved,
        Some(raw_headers(&format!(
            r#"{{"Authorization":"Basic {}"}}"#,
            STANDARD.encode("pk-lf:sk-lf")
        )))
    );
}

#[test]
fn langfuse_keys_resolve_environment_references() {
    let custom = raw_headers(
        r#"{"LANGFUSE_PUBLIC_KEY":"os.environ/LANGFUSE_PUB","LANGFUSE_SECRET_KEY":"os.environ/LANGFUSE_SEC"}"#,
    );
    let environment = environment(&[("LANGFUSE_PUB", "pk-lf"), ("LANGFUSE_SEC", "sk-lf")]);
    let resolved = set_env_variables_in_header(&secrets(), &environment, Some(&custom))
        .unwrap()
        .unwrap();
    assert_eq!(
        resolved.get("Authorization"),
        Some(&basic("pk-lf", "sk-lf"))
    );
}

#[test]
fn langfuse_embedded_reference_is_used_as_given() {
    let custom = raw_headers(
        r#"{"LANGFUSE_PUBLIC_KEY":"Bearer os.environ/LANGFUSE_PUB","LANGFUSE_SECRET_KEY":"sk-lf"}"#,
    );
    let environment = environment(&[("LANGFUSE_PUB", "pk-lf")]);
    let resolved = set_env_variables_in_header(&secrets(), &environment, Some(&custom))
        .unwrap()
        .unwrap();
    assert_eq!(
        resolved.get("Authorization"),
        Some(&basic("Bearer os.environ/LANGFUSE_PUB", "sk-lf"))
    );
}

#[test]
fn langfuse_public_key_without_secret_key_is_an_error() {
    let custom = raw_headers(r#"{"LANGFUSE_PUBLIC_KEY":"pk-lf"}"#);
    let error =
        set_env_variables_in_header(&secrets(), &BTreeMap::new(), Some(&custom)).unwrap_err();
    assert_eq!(
        error.to_string(),
        "pass-through headers must set LANGFUSE_PUBLIC_KEY and LANGFUSE_SECRET_KEY together; missing LANGFUSE_SECRET_KEY"
    );
}

#[test]
fn langfuse_secret_key_without_public_key_is_an_error() {
    let custom = raw_headers(r#"{"LANGFUSE_SECRET_KEY":"sk-lf"}"#);
    let error =
        set_env_variables_in_header(&secrets(), &BTreeMap::new(), Some(&custom)).unwrap_err();
    assert_eq!(
        error.to_string(),
        "pass-through headers must set LANGFUSE_PUBLIC_KEY and LANGFUSE_SECRET_KEY together; missing LANGFUSE_PUBLIC_KEY"
    );
}

#[test]
fn langfuse_unresolved_environment_reference_is_an_error() {
    let custom = raw_headers(
        r#"{"LANGFUSE_PUBLIC_KEY":"os.environ/NO_SUCH_PUBLIC","LANGFUSE_SECRET_KEY":"sk-lf"}"#,
    );
    let error =
        set_env_variables_in_header(&secrets(), &BTreeMap::new(), Some(&custom)).unwrap_err();
    assert_eq!(
        error.to_string(),
        "pass-through header LANGFUSE_PUBLIC_KEY references an environment variable that is not set"
    );
}

#[test]
fn langfuse_boolean_environment_value_is_not_text() {
    let custom = raw_headers(
        r#"{"LANGFUSE_PUBLIC_KEY":"os.environ/FEATURE_FLAG","LANGFUSE_SECRET_KEY":"sk-lf"}"#,
    );
    let environment = environment(&[("FEATURE_FLAG", "true")]);
    let error = set_env_variables_in_header(&secrets(), &environment, Some(&custom)).unwrap_err();
    assert_eq!(
        error.to_string(),
        "pass-through header LANGFUSE_PUBLIC_KEY references an environment variable that is not set"
    );
}

#[test]
fn langfuse_non_text_value_is_an_error() {
    let custom = raw_headers(r#"{"LANGFUSE_PUBLIC_KEY":3,"LANGFUSE_SECRET_KEY":"sk-lf"}"#);
    let error =
        set_env_variables_in_header(&secrets(), &BTreeMap::new(), Some(&custom)).unwrap_err();
    assert_eq!(
        error.to_string(),
        "pass-through header LANGFUSE_PUBLIC_KEY must be a string"
    );
}

#[test]
fn langfuse_authorization_takes_the_first_key_position() {
    let custom = raw_headers(
        r#"{"X-First":"1","LANGFUSE_PUBLIC_KEY":"pk-lf","X-Last":"2","LANGFUSE_SECRET_KEY":"sk-lf"}"#,
    );
    let resolved = set_env_variables_in_header(&secrets(), &BTreeMap::new(), Some(&custom))
        .unwrap()
        .unwrap();
    let keys: Vec<&str> = resolved.keys().map(String::as_str).collect();
    assert_eq!(keys, ["X-First", "Authorization", "X-Last"]);
    assert_eq!(resolved.get("X-First"), Some(&Json::Text("1".to_owned())));
    assert_eq!(resolved.get("X-Last"), Some(&Json::Text("2".to_owned())));
}

#[test]
fn langfuse_authorization_replaces_an_existing_header() {
    let custom = raw_headers(
        r#"{"Authorization":"Bearer old","LANGFUSE_PUBLIC_KEY":"pk-lf","LANGFUSE_SECRET_KEY":"sk-lf"}"#,
    );
    let resolved = set_env_variables_in_header(&secrets(), &BTreeMap::new(), Some(&custom))
        .unwrap()
        .unwrap();
    assert_eq!(resolved.len(), 1);
    assert_eq!(
        resolved.get("Authorization"),
        Some(&basic("pk-lf", "sk-lf"))
    );
}

#[test]
fn exact_route_registers_every_configured_method() {
    let routes = shared();
    let endpoint = parse_endpoint(
        r#"{"id":"e1","path":"/custom","target":"https://upstream","methods":["GET","POST"]}"#,
    );
    register_with(&routes, &[endpoint], None, &[]);
    let state = lock_state(&routes);
    let (key, metadata) = state
        .registry
        .registrations
        .iter()
        .find(|(key, _)| key == "e1:exact:/custom:GET,POST")
        .unwrap();
    assert_eq!(key, "e1:exact:/custom:GET,POST");
    assert_eq!(metadata.methods, ["GET".to_owned(), "POST".to_owned()]);
    assert_eq!(
        state
            .table
            .colliding_routes("/custom", &method_set(&["GET", "POST"]))
            .len(),
        1
    );
    assert!(state.endpoints.contains_key("e1:exact:/custom:GET,POST"));
}

#[test]
fn exact_route_defaults_to_all_five_methods() {
    let routes = shared();
    let endpoint = parse_endpoint(r#"{"id":"e1","path":"/custom","target":"https://upstream"}"#);
    register_with(&routes, &[endpoint], None, &[]);
    assert_eq!(
        registry_keys(&routes),
        ["e1:exact:/custom:DELETE,GET,PATCH,POST,PUT".to_owned()]
    );
    let state = lock_state(&routes);
    let (_, metadata) = state.registry.registrations.first().unwrap();
    assert_eq!(
        metadata.methods,
        [
            "GET".to_owned(),
            "POST".to_owned(),
            "PUT".to_owned(),
            "DELETE".to_owned(),
            "PATCH".to_owned()
        ]
    );
}

#[test]
fn exact_route_defaults_when_methods_are_empty() {
    let routes = shared();
    let endpoint =
        parse_endpoint(r#"{"id":"e1","path":"/custom","target":"https://upstream","methods":[]}"#);
    register_with(&routes, &[endpoint], None, &[]);
    assert_eq!(
        registry_keys(&routes),
        ["e1:exact:/custom:DELETE,GET,PATCH,POST,PUT".to_owned()]
    );
}

#[test]
fn exact_route_keeps_configured_method_text() {
    let routes = shared();
    let endpoint = parse_endpoint(
        r#"{"id":"e1","path":"/custom","target":"https://upstream","methods":["get"]}"#,
    );
    register_with(&routes, &[endpoint], None, &[]);
    let state = lock_state(&routes);
    assert_eq!(
        state
            .table
            .colliding_routes("/custom", &method_set(&["get"]))
            .len(),
        1
    );
    assert!(state.endpoints.contains_key("e1:exact:/custom:get"));
}

#[test]
fn exact_route_registers_another_method_set_on_the_same_path() {
    let routes = shared();
    let get = parse_endpoint(
        r#"{"id":"e1","path":"/custom","target":"https://upstream","methods":["GET"]}"#,
    );
    let post = parse_endpoint(
        r#"{"id":"e2","path":"/custom","target":"https://upstream","methods":["POST"]}"#,
    );
    register_with(&routes, &[get, post], None, &[]);
    assert_eq!(
        registry_keys(&routes),
        [
            "e1:exact:/custom:GET".to_owned(),
            "e2:exact:/custom:POST".to_owned()
        ]
    );
    let state = lock_state(&routes);
    assert_eq!(
        state
            .table
            .colliding_routes("/custom", &method_set(&["GET", "POST"]))
            .len(),
        2
    );
}

#[test]
fn exact_route_skips_a_path_a_static_route_holds() {
    let routes = shared_with_route("/custom", &["GET"]);
    let endpoint = parse_endpoint(
        r#"{"id":"e1","path":"/custom","target":"https://upstream","methods":["GET"]}"#,
    );
    register_with(&routes, &[endpoint], None, &[]);
    let state = lock_state(&routes);
    assert_eq!(
        state
            .table
            .colliding_routes("/custom", &method_set(&["GET"]))
            .len(),
        1
    );
    assert_eq!(
        state
            .registry
            .registrations
            .iter()
            .filter(|(key, _)| key == "e1:exact:/custom:GET")
            .count(),
        1
    );
    assert!(state.endpoints.contains_key("e1:exact:/custom:GET"));
}

#[test]
fn subpath_route_registers_the_wildcard_template() {
    let routes = shared();
    let endpoint = parse_endpoint(
        r#"{"id":"e1","path":"/custom","target":"https://upstream","include_subpath":true,"methods":["GET","POST"]}"#,
    );
    register_with(&routes, &[endpoint], None, &[]);
    let state = lock_state(&routes);
    let (_, metadata) = state
        .registry
        .registrations
        .iter()
        .find(|(key, _)| key == "e1:subpath:/custom:GET,POST")
        .unwrap();
    assert_eq!(metadata.route_type, PassThroughRouteType::Subpath);
    assert_eq!(metadata.path, "/custom");
    assert_eq!(
        state
            .table
            .colliding_routes("/custom/{subpath:path}", &method_set(&["GET"]))
            .len(),
        1
    );
}

#[test]
fn subpath_route_is_absent_without_include_subpath() {
    let routes = shared();
    let endpoint = parse_endpoint(r#"{"id":"e1","path":"/custom","target":"https://upstream"}"#);
    register_with(&routes, &[endpoint], None, &[]);
    let state = lock_state(&routes);
    assert_eq!(state.registry.registrations.len(), 1);
    assert!(
        state
            .table
            .colliding_routes("/custom/{subpath:path}", &method_set(&["GET"]))
            .is_empty()
    );
}

#[test]
fn subpath_route_ignores_a_string_true() {
    let routes = shared();
    let endpoint = parse_endpoint(
        r#"{"id":"e1","path":"/custom","target":"https://upstream","include_subpath":"true"}"#,
    );
    register_with(&routes, &[endpoint], None, &[]);
    assert_eq!(
        registry_keys(&routes),
        ["e1:exact:/custom:DELETE,GET,PATCH,POST,PUT".to_owned()]
    );
}

#[test]
fn subpath_route_ignores_a_null_include_subpath() {
    let routes = shared();
    let endpoint = parse_endpoint(
        r#"{"id":"e1","path":"/custom","target":"https://upstream","include_subpath":null}"#,
    );
    register_with(&routes, &[endpoint], None, &[]);
    assert_eq!(
        registry_keys(&routes),
        ["e1:exact:/custom:DELETE,GET,PATCH,POST,PUT".to_owned()]
    );
}

#[test]
fn auth_endpoint_adds_its_path_to_openai_routes() {
    let routes = shared();
    let endpoint = parse_endpoint(
        r#"{"id":"e1","path":"/custom","target":"https://upstream","auth":true,"methods":["GET"]}"#,
    );
    register_with(&routes, &[endpoint], None, &[]);
    assert_eq!(route_count(&routes, "/custom"), 1);
}

#[test]
fn auth_adds_the_path_once_across_reloads() {
    let routes = shared();
    let endpoint = parse_endpoint(
        r#"{"id":"e1","path":"/custom","target":"https://upstream","auth":true,"methods":["GET"]}"#,
    );
    register_with(&routes, &[endpoint.clone()], None, &[]);
    register_with(&routes, &[endpoint], None, &[]);
    assert_eq!(route_count(&routes, "/custom"), 1);
}

#[test]
fn auth_subpath_adds_the_wildcard_to_openai_routes() {
    let routes = shared();
    let endpoint = parse_endpoint(
        r#"{"id":"e1","path":"/custom","target":"https://upstream","auth":true,"include_subpath":true,"methods":["GET"]}"#,
    );
    register_with(&routes, &[endpoint], None, &[]);
    assert_eq!(route_count(&routes, "/custom"), 1);
    assert_eq!(route_count(&routes, "/custom/*"), 1);
}

#[test]
fn auth_subpath_without_auth_adds_no_routes() {
    let routes = shared();
    let endpoint = parse_endpoint(
        r#"{"id":"e1","path":"/custom","target":"https://upstream","include_subpath":true,"methods":["GET"]}"#,
    );
    register_with(&routes, &[endpoint], None, &[]);
    assert_eq!(route_count(&routes, "/custom"), 0);
    assert_eq!(route_count(&routes, "/custom/*"), 0);
}

#[test]
fn auth_string_true_is_enforced() {
    let routes = shared();
    let endpoint = parse_endpoint(
        r#"{"id":"e1","path":"/custom","target":"https://upstream","auth":"True","methods":["GET"]}"#,
    );
    register_with(&routes, &[endpoint], None, &[]);
    assert_eq!(route_count(&routes, "/custom"), 1);
    assert_eq!(metadata_auth(&routes, "e1:exact:/custom:GET"), Some(true));
}

#[test]
fn auth_string_false_is_not_enforced() {
    let routes = shared();
    let endpoint = parse_endpoint(
        r#"{"id":"e1","path":"/custom","target":"https://upstream","auth":"false","methods":["GET"]}"#,
    );
    register_with(&routes, &[endpoint], None, &[]);
    assert_eq!(route_count(&routes, "/custom"), 0);
    assert_eq!(metadata_auth(&routes, "e1:exact:/custom:GET"), Some(false));
}

#[test]
fn auth_boolean_false_is_not_enforced() {
    let routes = shared();
    let endpoint = parse_endpoint(
        r#"{"id":"e1","path":"/custom","target":"https://upstream","auth":false,"methods":["GET"]}"#,
    );
    register_with(&routes, &[endpoint], None, &[]);
    assert_eq!(route_count(&routes, "/custom"), 0);
}

#[test]
fn auth_wrong_type_is_an_error() {
    let routes = shared();
    let endpoint = parse_endpoint(
        r#"{"id":"e1","path":"/custom","target":"https://upstream","auth":1,"methods":["GET"]}"#,
    );
    let error = registration_error(&routes, &[endpoint], &[]);
    assert_eq!(error.to_string(), "auth must be a boolean or a string");
}

#[test]
fn auth_base_path_is_not_duplicated() {
    let routes = shared();
    let endpoint = parse_endpoint(
        r#"{"id":"e1","path":"/v1/chat/completions","target":"https://upstream","auth":true,"methods":["POST"]}"#,
    );
    register_with(&routes, &[endpoint], None, &[]);
    assert_eq!(route_count(&routes, "/v1/chat/completions"), 1);
}

#[test]
fn id_generation_fills_a_missing_id_with_a_uuid() {
    let routes = shared();
    let endpoint =
        parse_endpoint(r#"{"path":"/custom","target":"https://upstream","methods":["GET"]}"#);
    register_with(&routes, &[endpoint], None, &[]);
    let keys = registry_keys(&routes);
    let id = keys[0].split(':').next().unwrap();
    assert_eq!(Uuid::parse_str(id).unwrap().get_version_num(), 4);
}

#[test]
fn id_generation_keeps_a_given_id() {
    let routes = shared();
    let endpoint = parse_endpoint(
        r#"{"id":"given-id","path":"/custom","target":"https://upstream","methods":["GET"]}"#,
    );
    register_with(&routes, &[endpoint], None, &[]);
    assert_eq!(
        registry_keys(&routes),
        ["given-id:exact:/custom:GET".to_owned()]
    );
}

#[test]
fn id_generation_differs_for_two_endpoints_without_ids() {
    let routes = shared();
    let endpoint =
        parse_endpoint(r#"{"path":"/custom","target":"https://upstream","methods":["GET"]}"#);
    register_with(&routes, &[endpoint.clone(), endpoint], None, &[]);
    let keys = registry_keys(&routes);
    assert_eq!(keys.len(), 2);
    assert_ne!(
        keys[0].split(':').next().unwrap(),
        keys[1].split(':').next().unwrap()
    );
}

#[test]
fn order_preserves_the_supplied_endpoint_order() {
    let routes = shared();
    let endpoints = [
        parse_endpoint(r#"{"id":"a","path":"/a","target":"https://upstream","methods":["GET"]}"#),
        parse_endpoint(r#"{"id":"b","path":"/b","target":"https://upstream","methods":["GET"]}"#),
        parse_endpoint(r#"{"id":"c","path":"/c","target":"https://upstream","methods":["GET"]}"#),
    ];
    register_with(&routes, &endpoints, None, &[]);
    assert_eq!(
        registry_keys(&routes),
        [
            "a:exact:/a:GET".to_owned(),
            "b:exact:/b:GET".to_owned(),
            "c:exact:/c:GET".to_owned()
        ]
    );
}

#[test]
fn order_replaces_a_duplicate_key_in_place() {
    let routes = shared();
    let first =
        parse_endpoint(r#"{"id":"a","path":"/a","target":"https://first","methods":["GET"]}"#);
    let second =
        parse_endpoint(r#"{"id":"b","path":"/b","target":"https://upstream","methods":["GET"]}"#);
    let update =
        parse_endpoint(r#"{"id":"a","path":"/a","target":"https://second","methods":["GET"]}"#);
    register_with(&routes, &[first, second, update], None, &[]);
    assert_eq!(
        registry_keys(&routes),
        ["a:exact:/a:GET".to_owned(), "b:exact:/b:GET".to_owned()]
    );
    let state = lock_state(&routes);
    assert_eq!(
        state.endpoints.get("a:exact:/a:GET").unwrap().target,
        "https://second"
    );
}

#[test]
fn order_places_routes_ahead_of_lazy_routes_in_supplied_order() {
    let mut state = PassThroughRoutes::default();
    let lazy_a = state
        .table
        .install("/a".to_owned(), method_set(&["GET"]), true);
    let lazy_b = state
        .table
        .install("/b".to_owned(), method_set(&["GET"]), true);
    let routes = Arc::new(Mutex::new(state));
    let endpoints = [
        parse_endpoint(r#"{"id":"a","path":"/a","target":"https://upstream","methods":["GET"]}"#),
        parse_endpoint(r#"{"id":"b","path":"/b","target":"https://upstream","methods":["GET"]}"#),
    ];
    register_with(&routes, &endpoints, None, &[]);
    let state = lock_state(&routes);
    let order = state.table.order();
    assert_eq!(order.len(), 4);
    assert_eq!(order.iter().position(|id| *id == lazy_a), Some(1));
    assert_eq!(order.iter().position(|id| *id == lazy_b), Some(3));
}

#[test]
fn missing_path_returns_the_value_error() {
    let routes = shared();
    let endpoint = parse_endpoint(r#"{"target":"https://upstream"}"#);
    let error = registration_error(&routes, &[endpoint], &[]);
    assert_eq!(
        error.to_string(),
        "Path is required for pass-through endpoint"
    );
}

#[test]
fn missing_path_registers_nothing_for_that_endpoint() {
    let routes = shared();
    let good =
        parse_endpoint(r#"{"id":"e1","path":"/a","target":"https://upstream","methods":["GET"]}"#);
    let missing = parse_endpoint(r#"{"id":"e2","target":"https://upstream"}"#);
    let error = registration_error(&routes, &[good, missing], &[]);
    assert_eq!(
        error.to_string(),
        "Path is required for pass-through endpoint"
    );
    assert_eq!(registry_keys(&routes), ["e1:exact:/a:GET".to_owned()]);
    let state = lock_state(&routes);
    assert_eq!(state.endpoints.len(), 1);
    assert!(
        state
            .table
            .colliding_routes("/custom", &method_set(&["GET"]))
            .is_empty()
    );
}

#[test]
fn missing_path_stops_later_endpoints() {
    let routes = shared();
    let missing = parse_endpoint(r#"{"id":"e1","target":"https://upstream"}"#);
    let good =
        parse_endpoint(r#"{"id":"e2","path":"/b","target":"https://upstream","methods":["GET"]}"#);
    let error = registration_error(&routes, &[missing, good], &[]);
    assert_eq!(
        error.to_string(),
        "Path is required for pass-through endpoint"
    );
    assert!(registry_keys(&routes).is_empty());
    let state = lock_state(&routes);
    assert!(state.endpoints.is_empty());
    assert!(
        state
            .table
            .colliding_routes("/b", &method_set(&["GET"]))
            .is_empty()
    );
}

#[test]
fn missing_path_treats_null_as_absent() {
    let routes = shared();
    let endpoint = parse_endpoint(r#"{"id":"e1","path":null,"target":"https://upstream"}"#);
    let error = registration_error(&routes, &[endpoint], &[]);
    assert_eq!(
        error.to_string(),
        "Path is required for pass-through endpoint"
    );
}

#[test]
fn missing_target_keeps_the_openai_path_and_registers_no_route() {
    let routes = shared();
    let endpoint = parse_endpoint(r#"{"id":"e1","path":"/custom","auth":true,"methods":["GET"]}"#);
    register_with(&routes, &[endpoint], None, &[]);
    assert_eq!(route_count(&routes, "/custom"), 1);
    assert!(registry_keys(&routes).is_empty());
    let state = lock_state(&routes);
    assert!(state.endpoints.is_empty());
    assert!(
        state
            .table
            .colliding_routes("/custom", &method_set(&["GET"]))
            .is_empty()
    );
}

#[test]
fn missing_target_registers_no_subpath_route() {
    let routes = shared();
    let endpoint = parse_endpoint(
        r#"{"id":"e1","path":"/custom","auth":true,"include_subpath":true,"methods":["GET"]}"#,
    );
    register_with(&routes, &[endpoint], None, &[]);
    assert_eq!(route_count(&routes, "/custom"), 1);
    assert_eq!(route_count(&routes, "/custom/*"), 0);
    let state = lock_state(&routes);
    assert!(state.registry.registrations.is_empty());
    assert!(
        state
            .table
            .colliding_routes("/custom/{subpath:path}", &method_set(&["GET"]))
            .is_empty()
    );
}

#[test]
fn missing_target_leaves_earlier_endpoints_registered() {
    let routes = shared();
    let good =
        parse_endpoint(r#"{"id":"e1","path":"/a","target":"https://upstream","methods":["GET"]}"#);
    let no_target = parse_endpoint(r#"{"id":"e2","path":"/b","methods":["GET"]}"#);
    register_with(&routes, &[good, no_target], None, &[]);
    assert_eq!(registry_keys(&routes), ["e1:exact:/a:GET".to_owned()]);
}

#[test]
fn auth_default_is_unauthenticated_without_the_field() {
    let routes = shared();
    let endpoint = parse_endpoint(
        r#"{"id":"e1","path":"/custom","target":"https://upstream","methods":["GET"]}"#,
    );
    register_with(&routes, &[endpoint], None, &[]);
    assert_eq!(route_count(&routes, "/custom"), 0);
    assert_eq!(metadata_auth(&routes, "e1:exact:/custom:GET"), Some(false));
}

#[test]
fn auth_default_is_unauthenticated_when_null() {
    let routes = shared();
    let endpoint = parse_endpoint(
        r#"{"id":"e1","path":"/custom","target":"https://upstream","auth":null,"methods":["GET"]}"#,
    );
    register_with(&routes, &[endpoint], None, &[]);
    assert_eq!(route_count(&routes, "/custom"), 0);
    assert_eq!(metadata_auth(&routes, "e1:exact:/custom:GET"), Some(false));
}

#[test]
fn config_file_path_is_stored_on_the_exact_and_subpath_entries() {
    let routes = shared();
    let endpoint = parse_endpoint(
        r#"{"id":"e1","path":"/custom","target":"https://upstream","include_subpath":true,"methods":["GET"]}"#,
    );
    register_with(&routes, &[endpoint], Some("/etc/config.yaml"), &[]);
    assert_eq!(
        config_path(&routes, "e1:exact:/custom:GET").as_deref(),
        Some("/etc/config.yaml")
    );
    assert_eq!(
        config_path(&routes, "e1:subpath:/custom:GET").as_deref(),
        Some("/etc/config.yaml")
    );
}

#[test]
fn config_file_path_is_none_without_a_config_file() {
    let routes = shared();
    let endpoint = parse_endpoint(
        r#"{"id":"e1","path":"/custom","target":"https://upstream","methods":["GET"]}"#,
    );
    register_with(&routes, &[endpoint], None, &[]);
    assert_eq!(config_path(&routes, "e1:exact:/custom:GET"), None);
}

#[test]
fn config_file_path_stays_from_the_first_install_on_a_later_none_call() {
    let routes = shared();
    let endpoint = parse_endpoint(
        r#"{"id":"e1","path":"/custom","target":"https://upstream","methods":["GET"]}"#,
    );
    register_with(&routes, &[endpoint.clone()], Some("/etc/config.yaml"), &[]);
    register_with(&routes, &[endpoint], None, &[]);
    assert_eq!(
        config_path(&routes, "e1:exact:/custom:GET").as_deref(),
        Some("/etc/config.yaml")
    );
}

#[test]
fn config_file_path_keeps_the_first_install_value() {
    let routes = shared();
    let endpoint = parse_endpoint(
        r#"{"id":"e1","path":"/custom","target":"https://upstream","methods":["GET"]}"#,
    );
    register_with(&routes, &[endpoint.clone()], Some("/first.yaml"), &[]);
    register_with(&routes, &[endpoint], Some("/second.yaml"), &[]);
    assert_eq!(
        config_path(&routes, "e1:exact:/custom:GET").as_deref(),
        Some("/first.yaml")
    );
}

#[test]
fn config_file_path_uses_the_calls_value_when_a_static_route_holds_the_path() {
    let routes = shared_with_route("/custom", &["GET"]);
    let endpoint = parse_endpoint(
        r#"{"id":"e1","path":"/custom","target":"https://upstream","methods":["GET"]}"#,
    );
    register_with(&routes, &[endpoint.clone()], Some("/cfg.yaml"), &[]);
    assert_eq!(
        config_path(&routes, "e1:exact:/custom:GET").as_deref(),
        Some("/cfg.yaml")
    );
    register_with(&routes, &[endpoint], None, &[]);
    assert_eq!(config_path(&routes, "e1:exact:/custom:GET"), None);
}

#[test]
fn config_file_path_is_remembered_per_method_set() {
    let routes = shared();
    let get = parse_endpoint(
        r#"{"id":"e1","path":"/custom","target":"https://upstream","methods":["GET"]}"#,
    );
    let post = parse_endpoint(
        r#"{"id":"e1","path":"/custom","target":"https://upstream","methods":["POST"]}"#,
    );
    register_with(&routes, &[get.clone()], Some("/first.yaml"), &[]);
    register_with(&routes, &[post.clone()], Some("/second.yaml"), &[]);
    register_with(&routes, &[get, post], None, &[]);
    assert_eq!(
        config_path(&routes, "e1:exact:/custom:GET").as_deref(),
        Some("/first.yaml")
    );
    assert_eq!(
        config_path(&routes, "e1:exact:/custom:POST").as_deref(),
        Some("/second.yaml")
    );
}

#[test]
fn registered_entry_keeps_target_headers_and_options() {
    let routes = shared();
    let endpoint = parse_endpoint(
        r#"{"id":"e1","path":"/custom","target":"https://upstream.example","headers":{"Authorization":"os.environ/UP_KEY"},"forward_headers":true,"merge_query_params":false,"default_query_params":{"api-version":"2024-01"},"cost_per_request":0.25,"guardrails":{"pii":"on"},"timeout":12.5}"#,
    );
    register_with(&routes, &[endpoint], None, &[("UP_KEY", "sk-up")]);
    let state = lock_state(&routes);
    let stored = state
        .endpoints
        .get("e1:exact:/custom:DELETE,GET,PATCH,POST,PUT")
        .unwrap();
    assert_eq!(stored.target, "https://upstream.example");
    assert_eq!(
        stored.custom_headers,
        Some(raw_headers(r#"{"Authorization":"sk-up"}"#))
    );
    assert_eq!(stored.forward_headers, Some(true));
    assert_eq!(stored.merge_query_params, Some(false));
    assert_eq!(
        stored.default_query_params,
        Some(raw_headers(r#"{"api-version":"2024-01"}"#))
    );
    assert_eq!(stored.cost_per_request, Some(Decimal::new(25, 2)));
    assert_eq!(stored.guardrails, Some(raw_headers(r#"{"pii":"on"}"#)));
    assert_eq!(stored.timeout.map(f64::to_bits), Some(12.5_f64.to_bits()));
}

#[test]
fn default_handle_seeds_openai_routes_from_the_base_list() {
    let state = PassThroughRoutes::default();
    assert_eq!(state.openai_routes.len(), OPENAI_ROUTES.len());
    assert!(
        OPENAI_ROUTES
            .iter()
            .all(|route| state.openai_routes.iter().any(|entry| entry == route))
    );
}

fn direct_params() -> PassthroughParams {
    PassthroughParams {
        target: "https://upstream".to_owned(),
        custom_headers: None,
        forward_headers: None,
        merge_query_params: None,
        default_query_params: None,
        cost_per_request: None,
        guardrails: None,
        timeout: None,
        config_file_path: None,
    }
}

#[test]
fn direct_add_exact_path_route_registers_the_spec() {
    let routes = shared();
    let params = direct_params();
    let methods = ["GET".to_owned()];
    let spec = RouteSpec {
        endpoint_id: "e1",
        path: "/custom",
        methods: &methods,
        auth: true,
        params: &params,
    };
    add_exact_path_route(&routes, &spec, Some("/cfg.yaml"));
    assert_eq!(
        config_path(&routes, "e1:exact:/custom:GET").as_deref(),
        Some("/cfg.yaml")
    );
    let state = lock_state(&routes);
    assert_eq!(state.registry.registrations.len(), 1);
    assert_eq!(
        state
            .table
            .colliding_routes("/custom", &method_set(&["GET"]))
            .len(),
        1
    );
}

#[test]
fn direct_add_subpath_route_registers_the_wildcard() {
    let routes = shared();
    let params = direct_params();
    let methods = ["GET".to_owned()];
    let spec = RouteSpec {
        endpoint_id: "e1",
        path: "/custom",
        methods: &methods,
        auth: false,
        params: &params,
    };
    add_subpath_route(&routes, &spec, None);
    assert_eq!(config_path(&routes, "e1:subpath:/custom:GET"), None);
    let state = lock_state(&routes);
    assert_eq!(state.registry.registrations.len(), 1);
    assert_eq!(
        state.registry.registrations[0].1.route_type,
        PassThroughRouteType::Subpath
    );
    assert_eq!(
        state
            .table
            .colliding_routes("/custom/{subpath:path}", &method_set(&["GET"]))
            .len(),
        1
    );
}
