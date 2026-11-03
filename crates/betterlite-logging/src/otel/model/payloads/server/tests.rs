#![cfg(test)]

use super::*;
#[test]
fn server_preserves_explicit_ports_and_omits_scheme_defaults() {
    assert_eq!(
        ServerInfo::from_api_base(Some("https://example.com/path")).and_then(|info| info.port),
        None
    );
    assert_eq!(
        ServerInfo::from_api_base(Some("example.com:80")).and_then(|info| info.port),
        Some(80)
    );
    assert_eq!(
        ServerInfo::from_api_base(Some("https://[::1]:8443")).and_then(|info| info.address),
        Some("::1".to_owned())
    );
}
