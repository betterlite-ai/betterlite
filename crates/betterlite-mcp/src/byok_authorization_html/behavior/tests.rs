#![cfg(test)]
use super::{AuthorizePage, build_authorize_html};

fn page() -> AuthorizePage {
    AuthorizePage {
        server_name: "MCP".to_owned(),
        server_initial: "M".to_owned(),
        client_id: "id".to_owned(),
        redirect_uri: "https://return".to_owned(),
        code_challenge: "challenge".to_owned(),
        code_challenge_method: "S256".to_owned(),
        state: "state".to_owned(),
        server_id: "server".to_owned(),
        access_items: vec!["read".to_owned()],
        help_url: String::new(),
    }
}

#[test]
fn emits_two_step_form_with_navigation_and_key_input() {
    let html = build_authorize_html(&page());
    assert!(html.contains("id=\"s1\""));
    assert!(html.contains("id=\"s2\""));
    assert!(html.contains("name=\"api_key\""));
    assert!(html.contains("goStep2()"));
    assert!(html.contains("goStep1()"));
    assert!(html.contains("doCancel()"));
    assert!(html.contains("syncBtn()"));
    assert!(html.contains("access-item"));
}

#[test]
fn emits_duration_choices_and_keeps_until_revoked_default() {
    let html = build_authorize_html(&page());
    assert!(html.contains("toggleDur()"));
    assert!(html.contains("selDur('1h'"));
    assert!(html.contains("selDur('24h'"));
    assert!(html.contains("selDur('7d'"));
    assert!(html.contains("selDur('30d'"));
    assert!(html.contains("selDur('until_revoked'"));
    assert!(html.contains("value=\"until_revoked\""));
    assert!(!html.contains("value=\"24h\""));
}
