use betterlite_config::sections::provider_url_destination::pattern::AllowedDestination;
use betterlite_config::sections::request_body_safety::ClientsideParamGrant;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::request_body_safety::SafetyError;

use super::{Fixture, denied, denied_param};

fn allowlisted() -> Vec<AllowedDestination> {
    vec![AllowedDestination::parse("evil.example").expect("the host parses")]
}

#[test]
fn destination_opt_in_still_denied() {
    let mut fixture = Fixture::default();
    fixture.allow = true;
    fixture.hosts = allowlisted();
    // even the api_base matcher an admin configured cannot admit the destination
    let mut matcher = JsonObject::new();
    matcher.insert(
        "api_base".to_owned(),
        Json::Text("^https://evil.example".to_owned()),
    );
    fixture.grants.0.insert(
        "model".to_owned(),
        Some(vec![ClientsideParamGrant::Matcher(matcher)]),
    );
    assert_eq!(
        denied_param(&fixture, r#"{"api_base": "https://evil.example/v1"}"#),
        "api_base"
    );
    // the allowlisted observability host is refused by name with the same opt-ins in place
    assert_eq!(
        denied_param(&fixture, r#"{"langfuse_host": "https://evil.example"}"#),
        "langfuse_host"
    );
}

#[test]
fn fallback_allowlisted_host_denied() {
    let fixture = Fixture {
        hosts: allowlisted(),
        ..Fixture::default()
    };
    assert!(matches!(
        denied(&fixture, r#"{"fallbacks": ["https://evil.example/v1"]}"#),
        SafetyError::FallbackDestination
    ));
}
