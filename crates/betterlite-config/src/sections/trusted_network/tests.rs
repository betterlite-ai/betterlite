#![cfg(test)]

use std::collections::BTreeMap;

use super::Settings;
use crate::loader::{ConfigError, ConfigLoader};

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .trusted_network)
}

#[test]
fn loads_settings() {
    let source = "\
general_settings:
  trusted_proxy_ranges:
    - 10.0.0.0/8
    - os.environ/PROXY_CIDR
  mcp_trusted_proxy_ranges: [192.168.0.0/16, 172.16.0.0/12]
  use_x_forwarded_for: true
  mcp_xff_num_trusted_hops: os.environ/XFF_HOPS
";
    let settings = load(
        source,
        &[
            ("PROXY_CIDR", " 203.0.113.0/24 "),
            ("XFF_HOPS", "2"),
            ("PROXY_BASE_URL", "https://proxy.example.com"),
        ],
    )
    .unwrap();
    // the CIDR text stays as configured; the parser trims and skips an invalid entry at request time
    assert_eq!(
        settings.trusted_proxy_ranges,
        Some(vec!["10.0.0.0/8".to_owned(), " 203.0.113.0/24 ".to_owned()])
    );
    assert_eq!(
        settings.mcp_trusted_proxy_ranges,
        Some(vec![
            "192.168.0.0/16".to_owned(),
            "172.16.0.0/12".to_owned()
        ])
    );
    assert!(settings.use_x_forwarded_for);
    assert_eq!(settings.mcp_xff_num_trusted_hops, Some(2));
    assert_eq!(
        settings.proxy_base_url.as_deref(),
        Some("https://proxy.example.com")
    );
    // every setting above is consumed by betterlite-auth::trusted_network as this exact text or integer
    assert_ne!(settings, Settings::default());
}

#[test]
fn defaults_and_precedence() {
    let defaults = load("{}", &[]).unwrap();
    assert_eq!(defaults, Settings::default());
    assert!(!defaults.use_x_forwarded_for);
    assert_eq!(defaults.trusted_proxy_ranges, None);
    assert_eq!(defaults.mcp_trusted_proxy_ranges, None);
    assert_eq!(defaults.mcp_xff_num_trusted_hops, None);
    assert_eq!(defaults.proxy_base_url, None);

    // the two range settings are independent: an MCP range never falls back to the XFF range
    let settings = load(
        "general_settings:\n  trusted_proxy_ranges: [10.0.0.0/8]\n  use_x_forwarded_for: true\n",
        &[("PROXY_BASE_URL", "")],
    )
    .unwrap();
    assert_eq!(
        settings.trusted_proxy_ranges,
        Some(vec!["10.0.0.0/8".to_owned()])
    );
    assert_eq!(settings.mcp_trusted_proxy_ranges, None);
    assert!(settings.use_x_forwarded_for);
    // a variable that is set but empty stays Some(""), which is_request_https treats as unset
    assert_eq!(settings.proxy_base_url.as_deref(), Some(""));
}

#[test]
fn invalid_value_fallback() {
    // an entry that is not a CIDR loads as written; parse_trusted_proxy_ranges skips it and keeps the rest
    let settings = load(
        "general_settings:\n  trusted_proxy_ranges: [not-a-cidr, 10.0.0.0/8]\n",
        &[],
    )
    .unwrap();
    assert_eq!(
        settings.trusted_proxy_ranges,
        Some(vec!["not-a-cidr".to_owned(), "10.0.0.0/8".to_owned()])
    );

    // ProxyConfig declares list[str], so a scalar refuses the config
    let scalar = load(
        "general_settings:\n  trusted_proxy_ranges: 10.0.0.0/8\n",
        &[],
    )
    .unwrap_err();
    assert!(matches!(scalar, ConfigError::Yaml { .. }));
}

#[test]
fn invalid_hop_count_errors() {
    let zero = load("general_settings:\n  mcp_xff_num_trusted_hops: 0\n", &[]).unwrap_err();
    assert!(matches!(
        zero,
        ConfigError::InvalidScalar { path, .. }
            if path == "general_settings.mcp_xff_num_trusted_hops"
    ));

    let fraction = load("general_settings:\n  mcp_xff_num_trusted_hops: 1.5\n", &[]).unwrap_err();
    assert!(matches!(
        fraction,
        ConfigError::InvalidInteger { path, .. }
            if path == "general_settings.mcp_xff_num_trusted_hops"
    ));

    // one hop is the smallest value the boundary accepts; zero and below are refused above
    let one_hop = load("general_settings:\n  mcp_xff_num_trusted_hops: 1\n", &[]).unwrap();
    assert_eq!(one_hop.mcp_xff_num_trusted_hops, Some(1));
}

#[test]
fn missing_range_environment_names_the_path() {
    let missing = load(
        "general_settings:\n  mcp_trusted_proxy_ranges: [os.environ/NO_SUCH_RANGES]\n",
        &[],
    )
    .unwrap_err();
    assert!(matches!(
        missing,
        ConfigError::MissingEnvironment { variable, path }
            if variable == "NO_SUCH_RANGES"
                && path == "general_settings.mcp_trusted_proxy_ranges[0]"
    ));
}
