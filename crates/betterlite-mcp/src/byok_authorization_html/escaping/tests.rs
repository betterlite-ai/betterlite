#![cfg(test)]
use super::{AuthorizePage, build_authorize_html};
#[test]
fn escapes_dynamic_text_attributes_and_omits_empty_sections() {
    let html = build_authorize_html(&AuthorizePage {
        server_name: "<S&\"'".to_owned(),
        server_initial: "<I&\"'".to_owned(),
        client_id: "<id&\"'".to_owned(),
        redirect_uri: "r&\"'".to_owned(),
        code_challenge: "c<".to_owned(),
        code_challenge_method: "M>".to_owned(),
        state: "s\"".to_owned(),
        server_id: "id'".to_owned(),
        access_items: vec!["<read&\"'".to_owned()],
        help_url: "https://help.test/?x=1&y=2".to_owned(),
    });
    assert!(html.contains("&lt;S&amp;&quot;&#x27;"));
    assert!(html.contains("&lt;I&amp;&quot;&#x27;"));
    assert!(html.contains("&lt;id&amp;&quot;&#x27;"));
    assert!(html.contains("r&amp;&quot;&#x27;"));
    assert!(html.contains("c&lt;"));
    assert!(html.contains("M&gt;"));
    assert!(html.contains("s&quot;"));
    assert!(html.contains("id&#x27;"));
    assert!(html.contains("&lt;read&amp;&quot;&#x27;"));
    assert!(html.contains("href=\"https://help.test/?x=1&amp;y=2\""));
    let empty = build_authorize_html(&AuthorizePage {
        server_name: "S".to_owned(),
        server_initial: "S".to_owned(),
        client_id: String::new(),
        redirect_uri: String::new(),
        code_challenge: String::new(),
        code_challenge_method: String::new(),
        state: String::new(),
        server_id: String::new(),
        access_items: vec![],
        help_url: String::new(),
    });
    assert!(!empty.contains("access-box"));
    assert!(!empty.contains("help-link"));
}
