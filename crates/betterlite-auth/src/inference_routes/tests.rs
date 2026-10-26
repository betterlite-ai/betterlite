#![cfg(test)]

use rstest::rstest;

use super::{ANTHROPIC_ROUTES, OPENAI_ROUTES, llm_api_routes};

#[rstest]
#[case::chat(0, "/engines/{model}/chat/completions")]
#[case::batch_version_first(33, "/v1/batches")]
#[case::realtime_query(72, "/realtime?{model}")]
#[case::vector_store_singular(111, "/vector_store/list")]
fn literal_membership(#[case] index: usize, #[case] literal: &str) {
    assert_eq!(OPENAI_ROUTES.get(index), Some(&literal));
}

#[rstest]
#[case::unknown_path("/chat")]
#[case::wrong_query_spelling("/realtime?model={model}")]
#[case::fine_tuning_retrieve("/fine_tuning/jobs/{fine_tuning_job_id}")]
fn literal_nonmembership(#[case] literal: &str) {
    assert!(!OPENAI_ROUTES.contains(&literal));
}

#[test]
fn alias_membership() {
    assert!(OPENAI_ROUTES.contains(&"/v1/chat/completions"));
    assert!(OPENAI_ROUTES.contains(&"/openai/v1/responses"));
    assert!(ANTHROPIC_ROUTES.contains(&"/claude-code/plugins/{plugin_name}"));
}

#[test]
fn group_expansion() {
    let routes = llm_api_routes();
    assert_eq!(routes.len(), 251);
    assert_eq!(&routes[..OPENAI_ROUTES.len()], OPENAI_ROUTES);
    assert_eq!(routes[OPENAI_ROUTES.len()], "/v1/messages");
    assert!(routes.contains(&"/model/info"));
    assert!(routes.contains(&"/agents"));
    assert!(!routes.contains(&"/v1/mcp/server"));
}

#[test]
fn unknown_group_is_not_a_production_table() {
    assert!(!["mcp_routes", "agent_routes"].contains(&"llm_api_routes"));
}
