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

#[rstest::rstest]
#[case::empty("")]
#[case::path("/path")]
#[case::missing_authority("https:///path")]
#[case::invalid_port("https://host:bad")]
#[case::invalid_ipv6("https://[broken]")]
fn server_rejects_missing_or_invalid_hosts(#[case] raw: &str) {
    assert_eq!(ServerInfo::from_api_base(Some(raw)), None);
}

#[rstest::rstest]
#[case::https_default("https://example.com:443", "example.com", Some(443))]
#[case::http_default("http://example.com:80", "example.com", Some(80))]
#[case::custom_scheme("custom://Example.COM:9000/path", "example.com", Some(9000))]
#[case::credentials("https://name:pass@example.com/api", "example.com", None)]
#[case::unicode("https://b\u{fc}cher.example/api", "b\u{fc}cher.example", None)]
#[case::expanded_ipv6("https://[0:0:0:0:0:0:0:1]/api", "0:0:0:0:0:0:0:1", None)]
fn server_retains_host_and_explicit_port(
    #[case] raw: &str,
    #[case] host: &str,
    #[case] port: Option<u16>,
) {
    assert_eq!(
        ServerInfo::from_api_base(Some(raw)),
        Some(ServerInfo {
            address: Some(host.to_owned()),
            port
        })
    );
}
