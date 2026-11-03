#![cfg(test)]

use super::*;
#[test]
fn server_defaults_scheme_port() {
    assert_eq!(
        ServerInfo::from_api_base(Some("https://example.com/path")).and_then(|info| info.port),
        Some(443)
    );
}
