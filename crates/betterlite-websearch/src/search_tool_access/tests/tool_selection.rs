use crate::search_tool_access::tool_selection::{
    SearchToolConfig, SearchToolSelector, selected_search_tool_name,
};

fn config(json: &str) -> SearchToolConfig {
    serde_json::from_str(json).unwrap()
}

fn router_tools() -> Vec<SearchToolConfig> {
    vec![
        config(
            r#"{"search_tool_name": "first", "litellm_params": {"search_provider": "tavily", "api_key": "k1"}}"#,
        ),
        config(
            r#"{"search_tool_name": "second", "litellm_params": {"search_provider": "perplexity"}}"#,
        ),
    ]
}

#[test]
fn selects_the_entry_matching_the_configured_name() {
    let tools = router_tools();
    let selector = SearchToolSelector::new(Some("second".to_owned()));
    let selected = selector.select_from_router(Some(&tools));
    assert_eq!(
        selected.and_then(|tool| tool.search_tool_name.as_deref()),
        Some("second")
    );
    assert_eq!(
        selected.and_then(|tool| tool.search_provider.as_deref()),
        Some("perplexity")
    );
}

#[test]
fn falls_back_to_the_first_entry_when_the_name_matches_nothing() {
    let tools = router_tools();
    let selector = SearchToolSelector::new(Some("missing".to_owned()));
    let selected = selector.select_from_list(&tools);
    assert_eq!(
        selected.and_then(|tool| tool.search_tool_name.as_deref()),
        Some("first")
    );
}

#[test]
fn falls_back_to_the_first_entry_when_no_name_is_configured() {
    let tools = router_tools();
    let selector = SearchToolSelector::new(None);
    let selected = selector.select_from_list(&tools);
    assert_eq!(
        selected.and_then(|tool| tool.search_tool_name.as_deref()),
        Some("first")
    );
}

#[test]
fn falls_back_to_the_first_entry_when_the_configured_name_is_empty() {
    let tools = router_tools();
    let selector = SearchToolSelector::new(Some(String::new()));
    let selected = selector.select_from_list(&tools);
    assert_eq!(
        selected.and_then(|tool| tool.search_tool_name.as_deref()),
        Some("first")
    );
}

#[test]
fn returns_none_for_an_empty_list_and_an_absent_router() {
    let selector = SearchToolSelector::new(Some("first".to_owned()));
    assert!(selector.select_from_list(&[]).is_none());
    assert!(selector.select_from_router(Some(&[])).is_none());
    assert!(selector.select_from_router(None).is_none());
}

#[test]
fn reads_the_provider_out_of_litellm_params() {
    let entry = config(
        r#"{"search_tool_name": "s", "litellm_params": {"search_provider": "tavily", "api_key": "k1"}}"#,
    );
    assert_eq!(entry.search_provider.as_deref(), Some("tavily"));
    assert!(!entry.litellm_params.contains_key("search_provider"));
    assert_eq!(entry.litellm_params["api_key"].get(), "\"k1\"");
}

#[test]
fn a_null_search_provider_is_absent() {
    let entry = config(r#"{"search_tool_name": "s", "litellm_params": {"search_provider": null}}"#);
    assert!(entry.search_provider.is_none());
}

#[test]
fn rejects_a_non_string_search_provider() {
    let error = serde_json::from_str::<SearchToolConfig>(
        r#"{"search_tool_name": "s", "litellm_params": {"search_provider": 5}}"#,
    )
    .unwrap_err();
    assert!(error.to_string().contains("search_provider"));
}

#[test]
fn selected_name_is_none_for_a_missing_tool_missing_name_or_empty_name() {
    assert!(selected_search_tool_name(None).is_none());
    let missing = config(r#"{"litellm_params": {"search_provider": "tavily"}}"#);
    assert!(selected_search_tool_name(Some(&missing)).is_none());
    let empty =
        config(r#"{"search_tool_name": "", "litellm_params": {"search_provider": "tavily"}}"#);
    assert!(selected_search_tool_name(Some(&empty)).is_none());
}

#[test]
fn selected_name_returns_the_configured_name() {
    let named =
        config(r#"{"search_tool_name": "s", "litellm_params": {"search_provider": "tavily"}}"#);
    assert_eq!(selected_search_tool_name(Some(&named)), Some("s"));
}
