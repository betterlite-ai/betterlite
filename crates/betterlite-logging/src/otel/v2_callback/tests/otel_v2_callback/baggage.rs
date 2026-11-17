#![cfg(test)]

use std::collections::BTreeMap;

use opentelemetry::Context;
use opentelemetry::baggage::BaggageExt as _;

use betterlite_logging::otel::model::baggage::{TEAM_ALIAS, TEAM_METADATA};
use betterlite_logging::otel::model::metadata::wire::payload::Auth;
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use betterlite_logging::otel::plumbing::context::context_slots::RequestTraceContext;
use betterlite_logging::otel::v2_callback::identity::IdentitySeed;

use super::support::{attribute, harness, settings};

#[test]
fn a_trusted_auth_value_beats_the_requesters_metadata() {
    let mut config = settings();
    config.baggage_promoted_keys = Vec::new();
    config.baggage_metadata_keys = vec!["user_api_key_user_id".to_owned()];
    let harness = harness(config);
    let span = harness.span("server");
    let auth = Auth {
        user_id: Some("trusted".to_owned()),
        ..Auth::default()
    };
    let metadata = BTreeMap::from([(
        "user_api_key_user_id".to_owned(),
        OtelModelValue::String("requester".to_owned()),
    )]);
    let seed = IdentitySeed {
        auth: &auth,
        model: None,
        request_metadata: Some(&metadata),
        ambient_span: Some(&span),
    };
    let mut trace = RequestTraceContext::default();
    let seeded = harness
        .callback
        .seed_request_identity(&seed, &mut trace, &Context::new());
    assert_eq!(
        seeded
            .baggage()
            .get("litellm.metadata.user_api_key_user_id")
            .map(ToString::to_string),
        Some("trusted".to_owned())
    );
    span.end();
    let spans = harness.finished();
    assert_eq!(
        attribute(&spans[0], "litellm.metadata.user_api_key_user_id"),
        Some("trusted".to_owned())
    );
}

#[test]
fn a_malformed_metadata_value_promotes_nothing() {
    let mut config = settings();
    config.baggage_promoted_keys = Vec::new();
    config.baggage_metadata_keys = vec!["user_api_key_user_id".to_owned()];
    let harness = harness(config);
    let span = harness.span("server");
    let auth = Auth::default();
    let metadata = BTreeMap::from([(
        "user_api_key_user_id".to_owned(),
        OtelModelValue::Array(vec![OtelModelValue::Number(serde_json::Number::from(7))]),
    )]);
    let seed = IdentitySeed {
        auth: &auth,
        model: None,
        request_metadata: Some(&metadata),
        ambient_span: Some(&span),
    };
    let mut trace = RequestTraceContext::default();
    let seeded = harness
        .callback
        .seed_request_identity(&seed, &mut trace, &Context::new());
    assert!(
        seeded
            .baggage()
            .get("litellm.metadata.user_api_key_user_id")
            .is_none()
    );
    assert!(!span.has_attribute("litellm.metadata.user_api_key_user_id"));
}

#[test]
fn team_metadata_is_promoted_as_filtered_json() {
    let mut config = settings();
    config.baggage_promoted_keys = vec![TEAM_METADATA.to_owned()];
    config.baggage_team_metadata_keys = vec!["env".to_owned()];
    let harness = harness(config);
    let span = harness.span("server");
    let auth = Auth {
        team_metadata: Some(BTreeMap::from([
            ("env".to_owned(), OtelModelValue::String("prod".to_owned())),
            (
                "other".to_owned(),
                OtelModelValue::String("hidden".to_owned()),
            ),
        ])),
        ..Auth::default()
    };
    let seed = IdentitySeed {
        auth: &auth,
        model: None,
        request_metadata: None,
        ambient_span: Some(&span),
    };
    let mut trace = RequestTraceContext::default();
    let seeded = harness
        .callback
        .seed_request_identity(&seed, &mut trace, &Context::new());
    assert_eq!(
        seeded.baggage().get(TEAM_METADATA).map(ToString::to_string),
        Some("{\"env\":\"prod\"}".to_owned())
    );
}

#[test]
fn only_the_configured_promoted_keys_are_stamped() {
    let mut config = settings();
    config.baggage_promoted_keys = vec![TEAM_ALIAS.to_owned()];
    let harness = harness(config);
    let span = harness.span("server");
    let auth = Auth {
        team_id: Some("team-1".to_owned()),
        team_alias: Some("alias-1".to_owned()),
        ..Auth::default()
    };
    let seed = IdentitySeed {
        auth: &auth,
        model: None,
        request_metadata: None,
        ambient_span: Some(&span),
    };
    let mut trace = RequestTraceContext::default();
    let _seeded = harness
        .callback
        .seed_request_identity(&seed, &mut trace, &Context::new());
    assert!(span.has_attribute(TEAM_ALIAS));
    assert!(!span.has_attribute("litellm.team.id"));
}
