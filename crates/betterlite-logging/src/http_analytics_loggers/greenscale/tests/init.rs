use secrecy::ExposeSecret as _;

use crate::http_analytics_loggers::greenscale::GreenscaleSettings;
use crate::http_analytics_loggers::tests::support::environment;

#[test]
fn greenscale_reads_the_key_and_endpoint_at_init() {
    let settings = GreenscaleSettings::from_environment(&environment(&[
        ("GREENSCALE_API_KEY", "green-key"),
        (
            "GREENSCALE_ENDPOINT",
            "https://greenscale.example.com/v1/log",
        ),
    ]));
    assert_eq!(
        settings.api_key.expect("the key is set").expose_secret(),
        "green-key"
    );
    assert_eq!(
        settings.endpoint.as_deref(),
        Some("https://greenscale.example.com/v1/log")
    );
}

#[test]
fn greenscale_leaves_the_endpoint_unset_at_init_without_the_variable() {
    let settings =
        GreenscaleSettings::from_environment(&environment(&[("GREENSCALE_API_KEY", "green-key")]));
    assert!(settings.endpoint.is_none());
    assert!(settings.api_key.is_some());
}
