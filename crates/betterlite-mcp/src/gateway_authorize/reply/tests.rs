#![cfg(test)]

use super::*;
use crate::gateway_authorize::oauth_error;
use crate::gateway_dcr::oauth_error::{OauthError, OauthErrorCode};

fn cookie(name: &str, value: &str) -> (String, String) {
    (name.to_owned(), value.to_owned())
}

#[test]
fn the_view_reads_the_last_cookie_of_a_name() {
    let view = RequestView {
        base_url: "https://proxy.test".to_owned(),
        path: "/ui/connect".to_owned(),
        query: None,
        cookies: vec![
            cookie("mcp_connect_flow_h", "old"),
            cookie("other", "x"),
            cookie("mcp_connect_flow_h", "new"),
        ],
    };
    assert_eq!(view.cookie("mcp_connect_flow_h"), Some("new"));
    assert_eq!(view.cookie("other"), Some("x"));
    assert_eq!(view.cookie("missing"), None);
}

#[test]
fn a_redirect_reply_carries_no_cookies() {
    let reply = GatewayReply::redirect("https://proxy.test/ui/connect".to_owned());
    assert_eq!(
        reply,
        GatewayReply::Redirect {
            location: "https://proxy.test/ui/connect".to_owned(),
            set_cookie: None,
            delete_cookie: None,
        }
    );
}

#[test]
fn an_error_outcome_becomes_the_error_variant() {
    let reply = GatewayReply::from(Ok::<GatewayReply, OauthError>(GatewayReply::redirect(
        "https://proxy.test/x".to_owned(),
    )));
    assert!(matches!(reply, GatewayReply::Redirect { .. }));
    let failed = GatewayReply::from(Err::<GatewayReply, OauthError>(oauth_error(
        400,
        OauthErrorCode::InvalidRequest,
        "bad",
    )));
    let GatewayReply::Error(error) = failed else {
        panic!("expected an error");
    };
    assert_eq!(error.status, 400);
    assert_eq!(error.error, OauthErrorCode::InvalidRequest);
    assert_eq!(error.description, "bad");
}

#[test]
fn the_description_serializes_in_litellm_order() {
    let description = ConnectFlowDescription {
        state: ConnectFlowState::Interactive,
        client_origin: "http://127.0.0.1:8080".to_owned(),
        server_id: Some("srv-a".to_owned()),
        server_name: Some("alpha".to_owned()),
        connected: Some(true),
    };
    assert_eq!(
        serde_json::to_string(&description).unwrap(),
        r#"{"state":"interactive","client_origin":"http://127.0.0.1:8080","server_id":"srv-a","server_name":"alpha","connected":true}"#
    );
    let empty = ConnectFlowDescription {
        state: ConnectFlowState::Unscoped,
        client_origin: String::new(),
        server_id: None,
        server_name: None,
        connected: None,
    };
    assert_eq!(
        serde_json::to_string(&empty).unwrap(),
        r#"{"state":"unscoped","client_origin":"","server_id":null,"server_name":null,"connected":null}"#
    );
}
