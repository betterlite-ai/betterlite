#![cfg(test)]

use crate::byok_consent::get::{GetQuery, authorize_get};
use crate::byok_consent::test_support::{page_query as query, registry, server};
use crate::server_record::MCPServer;

fn page() -> String {
    authorize_get(&query(), None).unwrap()
}

fn github() -> MCPServer {
    server(
        r#"{"server_id": "srv-1", "name": "github", "server_name": "GitHub", "transport": "sse", "byok_description": ["Read repositories"], "byok_api_key_help_url": "https://github.com/settings/tokens"}"#,
    )
}

fn registry_page() -> String {
    let query = GetQuery {
        server_id: Some("srv-1".to_owned()),
        ..query()
    };
    authorize_get(&query, Some(&registry(vec![github()]))).unwrap()
}

#[test]
fn the_page_is_a_complete_html_document() {
    let html = page();
    assert!(html.starts_with("<!DOCTYPE html>"));
    assert!(html.ends_with("</html>"));
}

#[test]
fn the_form_posts_to_the_authorize_page() {
    let html = page();
    assert!(html.contains("<form method=\"POST\" id=\"authForm\""));
    assert!(html.contains("type=\"submit\""));
}

#[test]
fn the_hidden_fields_carry_the_request() {
    let html = page();
    assert!(html.contains("name=\"client_id\" value=\"client-1\""));
    assert!(html.contains("name=\"redirect_uri\" value=\"http://127.0.0.1:1456/callback\""));
    assert!(html.contains("name=\"code_challenge\" value=\"challenge\""));
    assert!(html.contains("name=\"code_challenge_method\" value=\"S256\""));
    assert!(html.contains("name=\"state\" value=\"state-1\""));
    assert!(html.contains("name=\"server_id\" value=\"\""));
}

#[test]
fn the_duration_defaults_to_until_revoked() {
    assert!(page().contains("name=\"duration\" id=\"durInput\" value=\"until_revoked\""));
}

#[test]
fn the_duration_pills_offer_litellms_five_choices() {
    let html = page();
    for duration in ["1h", "24h", "7d", "30d", "until_revoked"] {
        assert!(
            html.contains(&format!("selDur('{duration}',this)")),
            "the pill for {duration} is missing"
        );
    }
    assert!(html.contains("class=\"pill sel\" onclick=\"selDur('24h',this)\""));
}

#[test]
fn the_api_key_field_is_a_required_password_input() {
    let html = page();
    assert!(
        html.contains(
            "<input type=\"password\" name=\"api_key\" id=\"apiKey\" class=\"key-input\""
        )
    );
    assert!(html.contains("required autofocus"));
}

#[test]
fn the_page_has_both_steps() {
    let html = page();
    assert!(html.contains("<div id=\"s1\" class=\"step show\">"));
    assert!(html.contains("<div id=\"s2\" class=\"step\">"));
    assert!(html.contains("function goStep2()"));
    assert!(html.contains("function goStep1()"));
}

#[test]
fn the_page_names_the_server_in_its_title() {
    assert!(page().contains("<title>Connect MCP Server &mdash; LiteLLM</title>"));
    assert!(registry_page().contains("<title>Connect GitHub &mdash; LiteLLM</title>"));
}

#[test]
fn no_placeholder_survives_rendering() {
    let html = registry_page();
    for placeholder in [
        "{server}",
        "{initial}",
        "{client_id}",
        "{redirect_uri}",
        "{challenge}",
        "{method}",
        "{state}",
        "{server_id}",
        "{access}",
        "{help}",
    ] {
        assert!(!html.contains(placeholder), "{placeholder} was not filled");
    }
}

#[test]
fn the_help_link_and_access_items_come_from_the_registry() {
    let html = registry_page();
    assert!(html.contains("href=\"https://github.com/settings/tokens\""));
    assert!(html.contains("class=\"access-item\""));
    assert!(html.contains("Read repositories"));
    assert!(!page().contains("class=\"access-box\""));
}

#[test]
fn the_page_shows_the_proxy_logo_and_a_server_monogram() {
    let html = page();
    assert!(html.contains("class=\"logos\""));
    assert!(html.contains("src=\"/get_image?variant=monogram\""));
    assert!(html.contains("class=\"logo logo-s\">M<"));
}
