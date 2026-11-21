#![cfg(test)]

use rstest::rstest;

use super::{
    ClearCookie, STATE_COOKIE_HANDLE_BYTES, STATE_COOKIE_TTL_SECONDS, SameSite, StateCookie,
    clear_state_cookie, cookie_path_and_secure, resolve_encoded_state, set_state_cookie,
    state_cookie_name,
};

#[test]
fn state_cookie_name_prepends_the_prefix() {
    assert_eq!(state_cookie_name("handle"), "mcp_oauth_state_handle");
}

#[test]
fn cookie_constants_match_litellm() {
    assert_eq!(STATE_COOKIE_TTL_SECONDS, 600);
    assert_eq!(STATE_COOKIE_HANDLE_BYTES, 32);
}

#[rstest]
#[case::path_from_the_base_url("https://gateway.example/mcp/callback?x=1", "/mcp/callback", true)]
#[case::http_is_not_secure("http://gateway.example/mcp", "/mcp", false)]
#[case::empty_path_is_the_root("https://gateway.example", "/", true)]
#[case::root_path("https://gateway.example/", "/", true)]
#[case::relative_base_url("gateway.example/cb", "gateway.example/cb", false)]
#[case::invalid_port_keeps_the_path("https://gateway.example:bad/cb", "/cb", true)]
#[case::bracketed_ipv6_keeps_the_path("https://[::1]:8443/mcp", "/mcp", true)]
#[case::malformed_ipv6_falls_back("https://[::1/cb", "/", false)]
fn cookie_path_and_secure_matches_litellm(
    #[case] base_url: &str,
    #[case] path: &str,
    #[case] secure: bool,
) {
    assert_eq!(cookie_path_and_secure(base_url), (path.to_owned(), secure));
}

#[test]
fn set_state_cookie_writes_every_field() {
    assert_eq!(
        set_state_cookie("https://gateway.example/mcp", "handle", "encoded"),
        StateCookie {
            name: "mcp_oauth_state_handle".to_owned(),
            value: "encoded".to_owned(),
            max_age: 600,
            path: "/mcp".to_owned(),
            secure: true,
            http_only: true,
            same_site: SameSite::Lax,
        }
    );
}

#[test]
fn set_state_cookie_is_not_secure_over_http() {
    let cookie = set_state_cookie("http://gateway.example/mcp", "handle", "encoded");
    assert!(!cookie.secure);
    assert_eq!(cookie.path, "/mcp");
}

#[test]
fn resolve_encoded_state_prefers_the_cookie() {
    let cookies = |name: &str| {
        if name == "mcp_oauth_state_handle" {
            Some("encoded".to_owned())
        } else {
            None
        }
    };
    assert_eq!(resolve_encoded_state(&cookies, "handle"), "encoded");
}

#[test]
fn resolve_encoded_state_falls_back_to_the_state() {
    let absent = |_name: &str| None::<String>;
    assert_eq!(resolve_encoded_state(&absent, "state"), "state");
    let empty = |_name: &str| Some(String::new());
    assert_eq!(resolve_encoded_state(&empty, "state"), "state");
}

#[test]
fn resolve_encoded_state_ignores_another_handle() {
    let cookies = |name: &str| {
        if name == "mcp_oauth_state_other" {
            Some("encoded".to_owned())
        } else {
            None
        }
    };
    assert_eq!(resolve_encoded_state(&cookies, "handle"), "handle");
}

#[test]
fn clear_state_cookie_returns_nothing_without_the_cookie() {
    assert_eq!(
        clear_state_cookie("https://gateway.example/mcp", false, "handle"),
        None
    );
}

#[test]
fn clear_state_cookie_writes_the_delete_spec() {
    assert_eq!(
        clear_state_cookie("https://gateway.example/mcp", true, "handle"),
        Some(ClearCookie {
            name: "mcp_oauth_state_handle".to_owned(),
            path: "/mcp".to_owned(),
            secure: true,
            http_only: true,
            same_site: SameSite::Lax,
        })
    );
}

#[test]
fn clear_state_cookie_is_not_secure_over_http() {
    let cookie = clear_state_cookie("http://gateway.example/mcp", true, "handle").unwrap();
    assert!(!cookie.secure);
    assert_eq!(cookie.path, "/mcp");
}
