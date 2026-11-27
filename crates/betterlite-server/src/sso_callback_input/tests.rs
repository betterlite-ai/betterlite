#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use betterlite_auth::jwt_claims::claims::JwtClaims;
use betterlite_auth::jwt_claims::settings::JwtAuthSettings;
use betterlite_auth::jwt_claims::teams::TeamClaims as _;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::sso_callback_input::Settings;
use rstest::rstest;

use super::groups::sso_jwt_handler;
use super::query::{OAuthCallbackQuery, decode_oauth_callback_query};

const CLAIM_CONFIG: &str = "general_settings:\n  ui_access_mode:\n    type: restricted_sso_group\n    restricted_sso_group: admins\n    sso_group_jwt_field: groups\n";

fn loaded_settings(source: &str) -> Settings {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .expect("the config loads")
    .sections
    .sso_callback_input
}

#[test]
fn query_keeps_the_last_repeated_state() {
    let decoded = decode_oauth_callback_query(b"state=first&state=second&state=third");
    assert_eq!(decoded.state.as_deref(), Some("third"));
}

#[test]
fn query_reads_a_blank_error_as_an_empty_string_without_a_description() {
    let decoded = decode_oauth_callback_query(b"state=abc&error=");
    assert_eq!(decoded.error.as_deref(), Some(""));
    assert_eq!(decoded.error_description, None);
}

#[test]
fn query_reads_a_name_without_equals_as_a_blank_value() {
    let decoded = decode_oauth_callback_query(b"state&error&error_description");
    assert_eq!(decoded.state.as_deref(), Some(""));
    assert_eq!(decoded.error.as_deref(), Some(""));
    assert_eq!(decoded.error_description.as_deref(), Some(""));
}

#[test]
fn query_decodes_plus_and_percent_escapes() {
    let decoded = decode_oauth_callback_query(
        b"state=a+b%2Fc&error=invalid%20request&error_description=bad%2Bstate%3A+oops",
    );
    assert_eq!(decoded.state.as_deref(), Some("a b/c"));
    assert_eq!(decoded.error.as_deref(), Some("invalid request"));
    assert_eq!(
        decoded.error_description.as_deref(),
        Some("bad+state: oops")
    );
}

#[test]
fn query_keeps_a_malformed_escape_as_text() {
    let decoded = decode_oauth_callback_query(b"state=%2&error=%zz");
    assert_eq!(decoded.state.as_deref(), Some("%2"));
    assert_eq!(decoded.error.as_deref(), Some("%zz"));
}

#[test]
fn query_replaces_invalid_utf8_with_the_replacement_character() {
    let decoded = decode_oauth_callback_query(b"state=%C3%28&error=%FF");
    assert_eq!(decoded.state.as_deref(), Some("\u{FFFD}("));
    assert_eq!(decoded.error.as_deref(), Some("\u{FFFD}"));
}

#[test]
fn query_ignores_names_other_than_the_three_callback_fields() {
    let decoded = decode_oauth_callback_query(b"code=abc&scope=openid+email&session_state=xyz");
    assert_eq!(decoded, OAuthCallbackQuery::default());
}

#[rstest]
fn replays_the_oauth_callback_query_corpus(
    #[files("fuzz/corpus/oauth_callback_query/*")] path: PathBuf,
) {
    let bytes = fs::read(path).expect("the corpus seed is readable");
    drop(decode_oauth_callback_query(&bytes));
}

#[rstest]
#[case::unset("general_settings: {}\n")]
#[case::string_mode("general_settings:\n  ui_access_mode: admin_only\n")]
fn group_projection_builds_no_handler_without_a_mapping(#[case] source: &str) {
    assert!(sso_jwt_handler(&loaded_settings(source)).is_none());
}

#[test]
fn group_projection_mapping_builds_a_zero_leeway_handler_with_the_claim() {
    let handler =
        sso_jwt_handler(&loaded_settings(CLAIM_CONFIG)).expect("a mapping builds a handler");
    assert_eq!(
        handler.settings,
        JwtAuthSettings {
            team_id_jwt_field: None,
            team_ids_jwt_field: Some("groups".to_owned()),
            team_id_default: None,
            team_alias_jwt_field: None,
            org_id_jwt_field: None,
            org_alias_jwt_field: None,
            user_id_jwt_field: None,
            user_email_jwt_field: None,
            user_allowed_email_domain: None,
            user_id_upsert: false,
            end_user_id_jwt_field: None,
            agent_id_jwt_field: None,
            object_id_jwt_field: None,
        }
    );
    assert_eq!(handler.leeway, 0);
    assert!(handler.trusted_issuers.is_empty());
    assert!(handler.agent_lookup.is_none());
}

#[test]
fn group_projection_mapping_without_the_field_still_builds_a_handler() {
    let handler = sso_jwt_handler(&loaded_settings(
        "general_settings:\n  ui_access_mode:\n    type: restricted_sso_group\n    restricted_sso_group: admins\n",
    ))
    .expect("a mapping builds a handler");
    assert_eq!(handler.settings.team_ids_jwt_field, None);
}

#[test]
fn group_projection_feeds_the_configured_claim_into_team_ids() {
    let handler =
        sso_jwt_handler(&loaded_settings(CLAIM_CONFIG)).expect("a mapping builds a handler");
    let claims = JwtClaims {
        claims: serde_json::from_str(r#"{"groups":["team-a","team-b"],"teams":["ignored"]}"#)
            .expect("the claims are JSON"),
        settings: &handler.settings,
        trusted_issuers: handler.trusted_issuers.as_slice(),
    };
    assert_eq!(
        claims.team_ids().expect("the team claim reads"),
        vec!["team-a", "team-b"]
    );
}
