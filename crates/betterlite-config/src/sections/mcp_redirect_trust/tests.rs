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
        .mcp_redirect_trust)
}

#[test]
fn unset_variables_read_empty() {
    let source = "general_settings:\n  encryption_algorithm: aes-256-gcm\n";
    let settings = load(source, &[]).unwrap();
    assert_eq!(settings.trusted_origins, "");
    assert_eq!(settings.native_redirect_uris, "");
    assert_eq!(Settings::default(), settings);
}

#[test]
fn reads_both_variables() {
    let origins = " https://app.example.com, *.example.com:8443 ,App.Example.COM:443 ";
    let native = " myapp://host/oauth/* ,Cursor://Anysphere.Cursor-MCP/oauth/* ";
    let settings = load(
        "",
        &[
            ("MCP_TRUSTED_REDIRECT_ORIGINS", origins),
            ("MCP_TRUSTED_NATIVE_REDIRECT_URIS", native),
        ],
    )
    .unwrap();
    assert_eq!(settings.trusted_origins, origins);
    assert_eq!(settings.native_redirect_uris, native);
}

#[test]
fn empty_variables_read_empty() {
    let settings = load(
        "",
        &[
            ("MCP_TRUSTED_REDIRECT_ORIGINS", ""),
            ("MCP_TRUSTED_NATIVE_REDIRECT_URIS", ""),
        ],
    )
    .unwrap();
    assert_eq!(settings.trusted_origins, "");
    assert_eq!(settings.native_redirect_uris, "");
    assert_eq!(Settings::default(), settings);
}
