use secrecy::ExposeSecret as _;

use crate::http_analytics_loggers::promptlayer::PromptLayerSettings;
use crate::http_analytics_loggers::tests::support::environment;

#[test]
fn promptlayer_reads_the_key_at_init() {
    let settings =
        PromptLayerSettings::from_environment(&environment(&[("PROMPTLAYER_API_KEY", "pl-key")]));
    assert_eq!(
        settings.api_key.expect("the key is set").expose_secret(),
        "pl-key"
    );
}

#[test]
fn promptlayer_leaves_the_key_unset_at_init_without_the_variable() {
    let settings = PromptLayerSettings::from_environment(&environment(&[]));
    assert!(settings.api_key.is_none());
}
