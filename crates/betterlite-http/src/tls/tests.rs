#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::tls::apply_policy;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::tls::TlsPolicy;
use reqwest::ClientBuilder;

#[rstest]
#[case::default_user_agent("", None)]
#[case::custom_user_agent("LITELLM_USER_AGENT", Some("agent/1"))]
fn http_policy_tests_behavior(#[case] mode: &str, #[case] expected: Option<&str>) {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("")
    .unwrap();
    let environment = if mode.is_empty() {
        BTreeMap::new()
    } else {
        BTreeMap::from([(mode.to_owned(), expected.unwrap_or_default().to_owned())])
    };
    let policy = TlsPolicy::resolve(&config, &environment, None).unwrap();
    let builder = apply_policy(ClientBuilder::new(), &policy).unwrap();
    let _client = builder.build().unwrap();
    if let Some(value) = expected {
        assert_eq!(policy.user_agent, value);
    } else {
        assert!(policy.user_agent.starts_with("betterlite/"));
    }
}

#[rstest]
fn replays_the_http_policy_corpus(#[files("fuzz/corpus/http_policy/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let Ok(source) = String::from_utf8(bytes) else {
        return;
    };
    let parsed = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&source);
    if let Ok(config) = parsed {
        let _policy = TlsPolicy::resolve(&config, &BTreeMap::new(), None);
    }
}

mod ca_policy;
