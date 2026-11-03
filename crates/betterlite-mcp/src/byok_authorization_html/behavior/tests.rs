#![cfg(test)]
use super::{AuthorizePage, build_authorize_html};
#[test]
fn emits_two_step_form_and_duration_controls() {
    let html = build_authorize_html(&AuthorizePage {
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
    });
    assert!(html.contains("id=\"s1\""), "missing id="s1"");
    assert!(html.contains("id=\"s2\""), "missing id="s2"");
    assert!(html.contains("name=\"api_key\""), "missing name="api_key"");
    assert!(html.contains("goStep2()"), "missing goStep2()");
    assert!(html.contains("goStep1()"), "missing goStep1()");
    assert!(html.contains("doCancel()"), "missing doCancel()");
    assert!(html.contains("toggleDur()"), "missing toggleDur()");
    assert!(html.contains("selDur('1h'"), "missing selDur('1h'");
    assert!(html.contains("selDur('24h'"), "missing selDur('24h'");
    assert!(html.contains("selDur('7d'"), "missing selDur('7d'");
    assert!(html.contains("selDur('30d'"), "missing selDur('30d'");
    assert!(
        html.contains("selDur('until_revoked'"),
        "missing selDur('until_revoked'"
    );
    assert!(html.contains("value=\"until_revoked\""), "missing value="until_revoked"");
    assert!(html.contains("syncBtn()"), "missing syncBtn()");
    assert!(!html.contains("value=\"24h\""));
    assert!(html.contains("access-item"));
}
