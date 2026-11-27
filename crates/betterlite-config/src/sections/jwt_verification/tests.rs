#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::ConfigLoader;
use crate::types::Config;

use super::{AUDIENCE_VARIABLE, ISSUER_VARIABLE};

const DOCUMENT: &str = "model_list: []\n";

fn loaded(environment: &[(&str, &str)]) -> Config {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }.load(DOCUMENT).unwrap()
}

// Only absence disables audience and issuer validation; an empty value is still configured.
#[test]
fn global_scope_absent_vs_empty() {
    let absent = loaded(&[]);
    assert_eq!(absent.sections.jwt_verification.audience, None);
    assert_eq!(absent.sections.jwt_verification.issuer, None);

    let empty = loaded(&[(AUDIENCE_VARIABLE, ""), (ISSUER_VARIABLE, "")]);
    assert_eq!(
        empty.sections.jwt_verification.audience,
        Some(String::new())
    );
    assert_eq!(empty.sections.jwt_verification.issuer, Some(String::new()));
    assert!(empty.sections.jwt_verification.audience.is_some());
    assert!(empty.sections.jwt_verification.issuer.is_some());

    let scoped = loaded(&[
        (AUDIENCE_VARIABLE, "litellm-proxy"),
        (ISSUER_VARIABLE, "https://idp.example.com/"),
    ]);
    assert_eq!(
        scoped.sections.jwt_verification.audience.as_deref(),
        Some("litellm-proxy")
    );
    assert_eq!(
        scoped.sections.jwt_verification.issuer.as_deref(),
        Some("https://idp.example.com/")
    );

    // one scoped variable is enough to leave the unscoped pair's warning path
    let audience_only = loaded(&[(AUDIENCE_VARIABLE, "litellm-proxy")]);
    assert_eq!(
        audience_only.sections.jwt_verification.audience.as_deref(),
        Some("litellm-proxy")
    );
    assert_eq!(audience_only.sections.jwt_verification.issuer, None);
}
