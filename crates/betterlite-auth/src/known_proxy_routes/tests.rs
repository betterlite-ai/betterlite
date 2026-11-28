#![cfg(test)]

use rstest::rstest;

use super::{known_proxy_route, known_proxy_routes};

#[rstest]
#[case::openai("/chat/completions")]
#[case::mapped_pass_through("/bedrock")]
#[case::passthrough_wildcard("/bedrock/*")]
#[case::litellm_native("/rag/ingest")]
#[case::anthropic("/v1/messages")]
#[case::mcp_inference("/mcp")]
#[case::mcp_management("/v1/mcp/sessions")]
#[case::agent_inference("/a2a/{agent_id}/message/send")]
#[case::agent_management("/v1/agents/make_public")]
#[case::google("/v1beta/models/{model_name:path}:countTokens")]
#[case::apply_guardrail("/guardrails/apply_guardrail")]
#[case::model_info("/model_group/info")]
#[case::info("/key/health")]
#[case::master_key_only("/global/spend/reset")]
#[case::key_management("/key/generate")]
#[case::spend_tracking("/spend/keys")]
#[case::global_spend_tracking("/global/spend")]
#[case::public_routes("/routes")]
#[case::ui("/sso")]
#[case::compliance_check("/compliance/gdpr")]
#[case::internal_user_view_only("/tag/list")]
#[case::self_managed("/team/member_add")]
#[case::org_admin_only("/organization/member_delete")]
#[case::admin_viewer("/user/available_users")]
fn every_member_table_reaches_the_known_routes(#[case] route: &'static str) {
    assert_eq!(known_proxy_route(route), Some(route));
}

#[rstest]
#[case::chat_completion("chat_completion")]
#[case::completion("completion")]
#[case::embeddings("embeddings")]
#[case::image_generation("image_generation")]
#[case::video_generation("video_generation")]
#[case::audio_transcriptions("audio_transcriptions")]
#[case::moderations("moderations")]
#[case::model_list("model_list")]
fn openai_route_names_are_not_proxy_routes(#[case] name: &str) {
    assert_eq!(known_proxy_route(name), None);
}

#[rstest]
#[case::root("/")]
#[case::route_template("/key/{key_id}/reset_spend")]
#[case::guardrail_route("/guardrails/apply_guardrail")]
fn known_proxy_route_answers_the_exact_route(#[case] path: &'static str) {
    assert_eq!(known_proxy_route(path), Some(path));
}

// betterlite's anthropic table lists two routes LiteLLM serves but leaves out of LiteLLMRoutes, so they stay known
#[rstest]
#[case::plugin_enable("/claude-code/plugins/{plugin_name}/enable")]
#[case::plugin_disable("/claude-code/plugins/{plugin_name}/disable")]
fn plugin_routes_stay_known(#[case] route: &'static str) {
    assert_eq!(known_proxy_route(route), Some(route));
}

#[rstest]
#[case::concrete_key("/key/abc")]
#[case::trailing_slash("/chat/completions/")]
#[case::upper_case("/CHAT/COMPLETIONS")]
#[case::leading_space(" /chat/completions")]
#[case::empty("")]
#[case::unknown("/not/a/route")]
fn known_proxy_route_answers_nothing_for_other_text(#[case] path: &str) {
    assert_eq!(known_proxy_route(path), None);
}

#[test]
fn every_known_route_starts_with_a_slash() {
    let routes = known_proxy_routes();
    let stray = routes.iter().find(|route| !route.starts_with('/'));
    assert_eq!(stray, None);
}

#[test]
fn known_proxy_route_answers_every_known_route() {
    let routes = known_proxy_routes();
    let missed = routes
        .into_iter()
        .find(|route| known_proxy_route(route) != Some(*route));
    assert_eq!(missed, None);
}
