#![cfg(test)]
use super::{AuthorizePage, build_authorize_html};
#[test]
fn emits_named_oauth_fields_with_values() {
    let html = build_authorize_html(&AuthorizePage {
        server_name: "S".to_owned(),
        server_initial: "S".to_owned(),
        client_id: "id".to_owned(),
        redirect_uri: "https://r/?a=1&b=2".to_owned(),
        code_challenge: "c".to_owned(),
        code_challenge_method: "S256".to_owned(),
        state: "state".to_owned(),
        server_id: "sid".to_owned(),
        access_items: vec![],
        help_url: String::new(),
    });
    assert!(html.contains("name=\"client_id\" value=\"id\""));
    assert!(html.contains("name=\"redirect_uri\" value=\"https://r/?a=1&amp;b=2\""));
    assert!(html.contains("name=\"code_challenge\" value=\"c\""));
    assert!(html.contains("name=\"code_challenge_method\" value=\"S256\""));
    assert!(html.contains("name=\"state\" value=\"state\""));
    assert!(html.contains("name=\"server_id\" value=\"sid\""));
    assert!(!html.contains("name=\"api_key\" value="));
}
