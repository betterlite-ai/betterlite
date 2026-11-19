use super::*;

use axum::http::StatusCode;

use crate::model_listing::info::create_model_info_response;

#[test]
fn a_listed_model_reports_its_own_cost_map_entry() {
    let mut cost_map = FakeCostMap::default();
    cost_map.entries.insert(
        "gpt-4o".to_owned(),
        object(vec![
            ("mode", text("chat")),
            ("max_input_tokens", number(128_000)),
            ("max_output_tokens", number(16_384)),
        ]),
    );
    let groups = ExpectedGroups::default();
    let sources = info_sources(None, &cost_map, None, &groups);
    let response =
        create_model_info_response(&info_request("gpt-4o"), &sources).expect("the entry answers");
    assert_eq!(
        response,
        object(vec![
            ("id", text("gpt-4o")),
            ("object", text("model")),
            ("created", number(CREATED_AT)),
            ("owned_by", text("openai")),
            ("mode", text("chat")),
            ("max_input_tokens", number(128_000)),
            ("max_output_tokens", number(16_384)),
        ])
    );
}

#[test]
fn the_created_time_comes_from_the_server_configuration() {
    let cost_map = FakeCostMap::default();
    let groups = ExpectedGroups::default();
    let mut sources = info_sources(None, &cost_map, None, &groups);
    sources.created_at = 7;
    let response =
        create_model_info_response(&info_request("gpt-4o"), &sources).expect("the entry answers");
    assert_eq!(response.get("created"), Some(&number(7)));
}

#[test]
fn a_cost_map_that_cannot_answer_leaves_only_the_base_fields() {
    let mut cost_map = FakeCostMap::default();
    cost_map.failing.insert("gpt-4o".to_owned());
    let groups = ExpectedGroups::default();
    let sources = info_sources(None, &cost_map, None, &groups);
    let response =
        create_model_info_response(&info_request("gpt-4o"), &sources).expect("the entry answers");
    assert_eq!(
        response,
        object(vec![
            ("id", text("gpt-4o")),
            ("object", text("model")),
            ("created", number(CREATED_AT)),
            ("owned_by", text("openai")),
        ])
    );
}

#[test]
fn a_listing_without_cost_map_keys_resolves_the_listed_name() {
    let mut router = FakeRouter::default();
    router.listings.insert(
        "public".to_owned(),
        DeploymentListingInfo {
            cost_map_keys: Vec::new(),
            max_input_tokens: None,
            max_output_tokens: None,
        },
    );
    let mut cost_map = FakeCostMap::default();
    cost_map.entries.insert(
        "public".to_owned(),
        object(vec![("max_input_tokens", number(100))]),
    );
    let groups = ExpectedGroups::default();
    let sources = info_sources(Some(&router), &cost_map, None, &groups);
    let response =
        create_model_info_response(&info_request("public"), &sources).expect("the entry answers");
    assert_eq!(response.get("max_input_tokens"), Some(&number(100)));
}

#[test]
fn a_deployments_exact_entry_wins_over_a_generalized_listed_entry() {
    let mut router = FakeRouter::default();
    router.listings.insert(
        "public".to_owned(),
        DeploymentListingInfo {
            cost_map_keys: strings(&["openai/gpt-4o-2024-08-06"]),
            max_input_tokens: None,
            max_output_tokens: None,
        },
    );
    let mut cost_map = FakeCostMap::default();
    cost_map.entries.insert(
        "public".to_owned(),
        object(vec![
            ("key", text("public")),
            ("max_input_tokens", number(999)),
        ]),
    );
    cost_map.generalized.insert("public".to_owned());
    cost_map.entries.insert(
        "openai/gpt-4o-2024-08-06".to_owned(),
        object(vec![("max_input_tokens", number(100))]),
    );
    let groups = ExpectedGroups::default();
    let sources = info_sources(Some(&router), &cost_map, None, &groups);
    let response =
        create_model_info_response(&info_request("public"), &sources).expect("the entry answers");
    assert_eq!(response.get("max_input_tokens"), Some(&number(100)));
}

#[test]
fn a_deployment_entry_wins_over_an_exact_listed_entry() {
    let mut router = FakeRouter::default();
    router.listings.insert(
        "public".to_owned(),
        DeploymentListingInfo {
            cost_map_keys: strings(&["openai/gpt-4o-2024-08-06"]),
            max_input_tokens: None,
            max_output_tokens: None,
        },
    );
    let mut cost_map = FakeCostMap::default();
    cost_map.entries.insert(
        "public".to_owned(),
        object(vec![("max_input_tokens", number(999))]),
    );
    cost_map.entries.insert(
        "openai/gpt-4o-2024-08-06".to_owned(),
        object(vec![("max_input_tokens", number(100))]),
    );
    let groups = ExpectedGroups::default();
    let sources = info_sources(Some(&router), &cost_map, None, &groups);
    let response =
        create_model_info_response(&info_request("public"), &sources).expect("the entry answers");
    assert_eq!(response.get("max_input_tokens"), Some(&number(100)));
}

#[test]
fn a_generalized_deployment_entry_follows_the_listed_exact_entry() {
    let mut router = FakeRouter::default();
    router.listings.insert(
        "public".to_owned(),
        DeploymentListingInfo {
            cost_map_keys: strings(&["family-model"]),
            max_input_tokens: None,
            max_output_tokens: None,
        },
    );
    let mut cost_map = FakeCostMap::default();
    cost_map.entries.insert(
        "public".to_owned(),
        object(vec![("max_input_tokens", number(100))]),
    );
    cost_map.entries.insert(
        "family-model".to_owned(),
        object(vec![
            ("key", text("family")),
            ("max_input_tokens", number(999)),
        ]),
    );
    cost_map.generalized.insert("family".to_owned());
    let groups = ExpectedGroups::default();
    let sources = info_sources(Some(&router), &cost_map, None, &groups);
    let response =
        create_model_info_response(&info_request("public"), &sources).expect("the entry answers");
    assert_eq!(response.get("max_input_tokens"), Some(&number(100)));
}

#[test]
fn a_group_reports_the_widest_limit_any_deployment_declares() {
    let mut router = FakeRouter::default();
    router.listings.insert(
        "group".to_owned(),
        DeploymentListingInfo {
            cost_map_keys: strings(&["a", "b"]),
            max_input_tokens: None,
            max_output_tokens: None,
        },
    );
    let mut cost_map = FakeCostMap::default();
    cost_map.entries.insert(
        "a".to_owned(),
        object(vec![
            ("max_input_tokens", number(10)),
            ("max_output_tokens", number(5)),
        ]),
    );
    cost_map.entries.insert(
        "b".to_owned(),
        object(vec![
            ("max_input_tokens", number(20)),
            ("max_output_tokens", number(7)),
        ]),
    );
    let groups = ExpectedGroups::default();
    let sources = info_sources(Some(&router), &cost_map, None, &groups);
    let response =
        create_model_info_response(&info_request("group"), &sources).expect("the entry answers");
    assert_eq!(response.get("max_input_tokens"), Some(&number(20)));
    assert_eq!(response.get("max_output_tokens"), Some(&number(7)));
}

#[test]
fn the_listing_info_limits_override_the_entries() {
    let mut router = FakeRouter::default();
    router.listings.insert(
        "group".to_owned(),
        DeploymentListingInfo {
            cost_map_keys: strings(&["a"]),
            max_input_tokens: Some(300),
            max_output_tokens: None,
        },
    );
    let mut cost_map = FakeCostMap::default();
    cost_map.entries.insert(
        "a".to_owned(),
        object(vec![
            ("max_input_tokens", number(100)),
            ("max_output_tokens", number(50)),
        ]),
    );
    let groups = ExpectedGroups::default();
    let sources = info_sources(Some(&router), &cost_map, None, &groups);
    let response =
        create_model_info_response(&info_request("group"), &sources).expect("the entry answers");
    assert_eq!(response.get("max_input_tokens"), Some(&number(300)));
    assert_eq!(response.get("max_output_tokens"), Some(&number(50)));
}

#[test]
fn the_configured_mode_overrides_the_entry_mode() {
    let mut router = FakeRouter::default();
    router
        .modes
        .insert("gpt-4o".to_owned(), "completion".to_owned());
    let mut cost_map = FakeCostMap::default();
    cost_map
        .entries
        .insert("gpt-4o".to_owned(), object(vec![("mode", text("chat"))]));
    let groups = ExpectedGroups::default();
    let sources = info_sources(Some(&router), &cost_map, None, &groups);
    let response =
        create_model_info_response(&info_request("gpt-4o"), &sources).expect("the entry answers");
    assert_eq!(response.get("mode"), Some(&text("completion")));
}

#[test]
fn an_alias_resolves_the_lookup_model_and_keeps_the_listed_id() {
    let mut router = FakeRouter::default();
    router
        .aliases
        .insert("public".to_owned(), "openai/gpt-4o".to_owned());
    router.listings.insert(
        "openai/gpt-4o".to_owned(),
        DeploymentListingInfo {
            cost_map_keys: strings(&["openai/gpt-4o-2024-08-06"]),
            max_input_tokens: None,
            max_output_tokens: None,
        },
    );
    let mut cost_map = FakeCostMap::default();
    cost_map.entries.insert(
        "openai/gpt-4o-2024-08-06".to_owned(),
        object(vec![("max_input_tokens", number(42))]),
    );
    let groups = ExpectedGroups::default();
    let sources = info_sources(Some(&router), &cost_map, None, &groups);
    let response =
        create_model_info_response(&info_request("public"), &sources).expect("the entry answers");
    assert_eq!(response.get("id"), Some(&text("public")));
    assert_eq!(response.get("max_input_tokens"), Some(&number(42)));
}

fn fallback_settings() -> Settings {
    let general = vec![FallbackChainEntry::Target(FallbackTarget::Model(
        "general-backup".to_owned(),
    ))];
    let window = vec![FallbackChainEntry::Target(FallbackTarget::Model(
        "window-backup".to_owned(),
    ))];
    Settings {
        fallbacks: general,
        context_window_fallbacks: window,
        content_policy_fallbacks: Vec::new(),
        max_fallbacks: 1,
    }
}

#[test]
fn metadata_fallbacks_use_the_general_chain_by_default() {
    let cost_map = FakeCostMap::default();
    let settings = fallback_settings();
    let groups = ExpectedGroups {
        chain: settings.fallbacks.clone(),
        answer: strings(&["general-backup"]),
    };
    let sources = info_sources(None, &cost_map, Some(&settings), &groups);
    let mut request = info_request("gpt-4o");
    request.include_metadata = true;
    let response = create_model_info_response(&request, &sources).expect("the entry answers");
    assert_eq!(
        response.get("metadata"),
        Some(&Json::Object(object(vec![(
            "fallbacks",
            Json::List(vec![text("general-backup")])
        )])))
    );
}

#[test]
fn metadata_fallbacks_follow_the_requested_type() {
    let cost_map = FakeCostMap::default();
    let settings = fallback_settings();
    let groups = ExpectedGroups {
        chain: settings.context_window_fallbacks.clone(),
        answer: strings(&["window-backup"]),
    };
    let sources = info_sources(None, &cost_map, Some(&settings), &groups);
    let mut request = info_request("gpt-4o");
    request.include_metadata = true;
    request.fallback_type = Some("context_window");
    let response = create_model_info_response(&request, &sources).expect("the entry answers");
    assert_eq!(
        response.get("metadata"),
        Some(&Json::Object(object(vec![(
            "fallbacks",
            Json::List(vec![text("window-backup")])
        )])))
    );
}

#[test]
fn metadata_without_a_configured_plan_lists_no_fallbacks() {
    let cost_map = FakeCostMap::default();
    let groups = ExpectedGroups::default();
    let sources = info_sources(None, &cost_map, None, &groups);
    let mut request = info_request("gpt-4o");
    request.include_metadata = true;
    let response = create_model_info_response(&request, &sources).expect("the entry answers");
    assert_eq!(
        response.get("metadata"),
        Some(&Json::Object(object(vec![(
            "fallbacks",
            Json::List(Vec::new())
        )])))
    );
}

#[test]
fn an_unknown_fallback_type_is_a_bad_request() {
    let cost_map = FakeCostMap::default();
    let settings = fallback_settings();
    let groups = ExpectedGroups::default();
    let sources = info_sources(None, &cost_map, Some(&settings), &groups);
    let mut request = info_request("gpt-4o");
    request.include_metadata = true;
    request.fallback_type = Some("nope");
    let error = create_model_info_response(&request, &sources).expect_err("the type is refused");
    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        error.to_string(),
        "Invalid fallback_type. Must be one of: ['general', 'context_window', 'content_policy']"
    );
}

#[test]
fn an_unknown_fallback_type_without_metadata_still_answers() {
    let cost_map = FakeCostMap::default();
    let groups = ExpectedGroups::default();
    let sources = info_sources(None, &cost_map, None, &groups);
    let mut request = info_request("gpt-4o");
    request.fallback_type = Some("nope");
    let response = create_model_info_response(&request, &sources).expect("the entry answers");
    assert_eq!(response.get("metadata"), None);
}
