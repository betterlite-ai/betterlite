use std::sync::Arc;

use crate::jwt_claims::handler::{AgentLookup, JwtHandler};
use crate::jwt_claims::settings::JwtAuthSettings;

use super::support::empty_settings;

struct FakeAgentRegistry;

impl AgentLookup for FakeAgentRegistry {}

#[test]
fn defaults_match_the_jwt_handler_constructor() {
    let handler = JwtHandler::default();
    assert_eq!(handler.leeway, 0);
    assert_eq!(handler.settings, JwtAuthSettings::default());
    assert!(handler.trusted_issuers.is_empty());
    assert!(handler.agent_lookup.is_none());
}

#[test]
fn update_environment_stores_the_resolved_values() {
    let mut handler = JwtHandler::default();
    let mut settings = empty_settings();
    settings.user_id_upsert = true;
    handler.update_environment(settings.clone(), vec!["https://idp.example".to_owned()], 30);
    assert_eq!(handler.settings, settings);
    assert_eq!(handler.trusted_issuers, vec!["https://idp.example"]);
    assert_eq!(handler.leeway, 30);
}

#[test]
fn binding_an_agent_lookup_replaces_the_default() {
    let handler = JwtHandler::default().bind_agent_lookup(Arc::new(FakeAgentRegistry));
    assert!(handler.agent_lookup.is_some());
}
