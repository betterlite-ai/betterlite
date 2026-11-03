#![cfg(test)]
use super::{AuthorizePage, build_authorize_html};
fn page(url: &str) -> AuthorizePage {
    AuthorizePage {
        server_name: "S".to_owned(),
        server_initial: "S".to_owned(),
        client_id: String::new(),
        redirect_uri: String::new(),
        code_challenge: String::new(),
        code_challenge_method: String::new(),
        state: String::new(),
        server_id: String::new(),
        access_items: vec![],
        help_url: url.to_owned(),
    }
}
#[rstest::rstest]
#[case::javascript("javascript:alert(1)")]
#[case::mixed_case("JaVaScRiPt:alert(1)")]
#[case::data("data:text/html,hi")]
#[case::vbscript("vbscript:msgbox(1)")]
#[case::leading_space("  javascript:alert(1)")]
#[case::embedded_control("java\nscript:alert(1)")]
fn rejects_active_schemes(#[case] input: &str) {
    let html = build_authorize_html(&page(input));
    assert!(!html.contains("help-link"));
    assert!(html.contains("name=\"api_key\""));
}
#[rstest::rstest]
#[case::https("https://help.test/key")]
#[case::relative("/help/key")]
fn keeps_safe_links(#[case] input: &str) {
    let html = build_authorize_html(&page(input));
    assert!(html.contains("help-link"));
    assert!(html.contains("name=\"api_key\""));
}
