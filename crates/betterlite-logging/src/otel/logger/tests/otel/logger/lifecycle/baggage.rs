use std::collections::BTreeMap;

use betterlite_logging::otel::logger::call_context::seed_identity_baggage;
use betterlite_logging::otel::model::payloads::RequestIdentity;
use opentelemetry::baggage::BaggageExt as _;
use opentelemetry::{Context, StringValue};

use crate::otel::support::with_exporters;

#[test]
fn baggage_promotes_only_the_allowlisted_identity_keys() {
    let mut settings = with_exporters(Vec::new());
    settings.baggage_promoted_keys = vec!["litellm.team.id".to_owned()];
    settings.baggage_metadata_keys = vec!["route".to_owned()];
    let identity = RequestIdentity {
        call_id: Some("call-1".to_owned()),
        team_id: Some("team-a".to_owned()),
        team_alias: None,
        team_metadata: None,
        key_hash: Some("hash".to_owned()),
        end_user: None,
        provider_model: None,
        request_route: None,
        metadata: BTreeMap::from([("route".to_owned(), "chat".to_owned())]),
    };
    let context = seed_identity_baggage(&settings, &identity, Some("gpt-4o"), Context::new());
    let bag = context.baggage();
    assert_eq!(
        bag.get("litellm.team.id").map(StringValue::as_str),
        Some("team-a")
    );
    assert_eq!(
        bag.get("litellm.metadata.route").map(StringValue::as_str),
        Some("chat")
    );
    // present on the identity, but not on either allowlist
    assert!(bag.get("litellm.api_key.hash").is_none());
    assert!(bag.get("gen_ai.request.model").is_none());
}
