use betterlite_models::decisions::Json;

use super::support::{
    Fixture, MODEL_GROUP, encoded_previous_response_id, hook_request, metadata, model_info, number,
    pin, pool, request, seed_shared_value, session_pin, settings, settings_with_ttl, shared_store,
    text, user_pin,
};
use crate::affinity_claims::error::AffinityError;
use crate::affinity_claims::identity::AffinityFlag;
use crate::affinity_claims::pin::StoredPin;
use crate::deployment_affinity::error::DeploymentAffinityError;
use crate::routing_state_store::test_support::FakeRedis;
use crate::routing_value_store::RouteValue;

// an empty pool has no stable model map key and passes through
pub(super) async fn empty_pool() {
    let fixture = Fixture::new(settings(&[AffinityFlag::Deployment]), &[]);
    fixture.seed(&user_pin("key"), &pin("a"), 600);
    let kwargs = metadata(&[("user_api_key_hash", text("key"))]);
    assert!(fixture.filter(&[], Some(&kwargs)).await.is_empty());
}

// an empty previous_response_id is present but decodes to no model id
pub(super) async fn empty_previous_response_id() {
    let fixture = Fixture::new(settings(&[AffinityFlag::ResponsesApi]), &[]);
    let healthy = pool(&["a", "b"]);
    let kwargs = request(&[("previous_response_id", text(""))]);
    assert_eq!(fixture.filter(&healthy, Some(&kwargs)).await, ["a", "b"]);
}

// a previous_response_id of another type names the field instead of being read as text
pub(super) async fn non_text_previous_response_id() {
    let fixture = Fixture::new(settings(&[AffinityFlag::ResponsesApi]), &[]);
    let healthy = pool(&["a", "b"]);
    let kwargs = request(&[("previous_response_id", number(7))]);
    assert!(matches!(
        fixture.filter_error(&healthy, Some(&kwargs)).await,
        DeploymentAffinityError::RequestNotText { field } if field == "previous_response_id"
    ));
}

// a null previous_response_id is absent, not an error
pub(super) async fn null_previous_response_id() {
    let fixture = Fixture::new(settings(&[AffinityFlag::ResponsesApi]), &[]);
    let healthy = pool(&["a", "b"]);
    let kwargs = request(&[("previous_response_id", Json::Null)]);
    assert_eq!(fixture.filter(&healthy, Some(&kwargs)).await, ["a", "b"]);
}

// an encoded previous_response_id of a deployment outside the pool is not selected
pub(super) async fn previous_response_outside_pool() {
    let fixture = Fixture::new(settings(&[AffinityFlag::ResponsesApi]), &[]);
    let healthy = pool(&["a"]);
    let kwargs = request(&[(
        "previous_response_id",
        text(&encoded_previous_response_id("b")),
    )]);
    assert_eq!(fixture.filter(&healthy, Some(&kwargs)).await, ["a"]);
}

// a caller identity of another type names the metadata field
pub(super) async fn non_text_user_key() {
    let fixture = Fixture::new(settings(&[AffinityFlag::Deployment]), &[]);
    let healthy = pool(&["a", "b"]);
    let kwargs = metadata(&[("user_api_key_hash", Json::Bool(true))]);
    assert!(matches!(
        fixture.filter_error(&healthy, Some(&kwargs)).await,
        DeploymentAffinityError::Affinity(AffinityError::MetadataNotText { field })
            if field == "user_api_key_hash"
    ));
}

// the JWT caller identity takes the namespaced user id
pub(super) async fn user_id_identity() {
    let fixture = Fixture::new(settings(&[AffinityFlag::Deployment]), &[]);
    fixture.seed(&user_pin("user_id:u1"), &pin("a"), 600);
    let healthy = pool(&["a", "b"]);
    let kwargs = metadata(&[("user_api_key_user_id", text("u1"))]);
    assert_eq!(fixture.filter(&healthy, Some(&kwargs)).await, ["a"]);
}

// a generated session id never activates session affinity
pub(super) async fn generated_session_ignored() {
    let fixture = Fixture::new(settings(&[AffinityFlag::Session]), &[]);
    fixture.seed(&session_pin("s1"), &pin("a"), 600);
    let healthy = pool(&["a", "b"]);
    let kwargs = metadata(&[
        ("session_id", text("s1")),
        ("litellm_session_id_generated", Json::Bool(true)),
    ]);
    assert_eq!(fixture.filter(&healthy, Some(&kwargs)).await, ["a", "b"]);
}

// a generated-session flag of another type names the metadata field
pub(super) async fn generated_flag_not_bool() {
    let fixture = Fixture::new(settings(&[AffinityFlag::Session]), &[]);
    let healthy = pool(&["a", "b"]);
    let kwargs = metadata(&[
        ("session_id", text("s1")),
        ("litellm_session_id_generated", text("yes")),
    ]);
    assert!(matches!(
        fixture.filter_error(&healthy, Some(&kwargs)).await,
        DeploymentAffinityError::Affinity(AffinityError::MetadataNotBool { field })
            if field == "litellm_session_id_generated"
    ));
}

// a zero or negative marker is absent, so it cannot activate session affinity
pub(super) async fn marker_zero_ignored() {
    let fixture = Fixture::new(settings(&[AffinityFlag::Deployment]), &[]);
    fixture.seed(&session_pin("s1"), &pin("a"), 600);
    let healthy = pool(&["a", "b"]);
    let kwargs = metadata(&[
        ("session_id", text("s1")),
        ("_session_deployment_affinity_ttl", number(0)),
    ]);
    assert_eq!(fixture.filter(&healthy, Some(&kwargs)).await, ["a", "b"]);
}

// a marker past the ttl range names the metadata field
pub(super) async fn marker_out_of_range() {
    let fixture = Fixture::new(settings(&[AffinityFlag::Deployment]), &[]);
    let healthy = pool(&["a", "b"]);
    let kwargs = metadata(&[
        ("session_id", text("s1")),
        ("_session_deployment_affinity_ttl", number(4_294_967_296)),
    ]);
    assert!(matches!(
        fixture.filter_error(&healthy, Some(&kwargs)).await,
        DeploymentAffinityError::Affinity(AffinityError::TtlOutOfRange { field })
            if field == "_session_deployment_affinity_ttl"
    ));
}

// a shared value of another route type is not a pin and selects nothing
pub(super) async fn shared_value_not_a_pin_ignored() {
    let server = FakeRedis::start().await;
    let store = shared_store(&server);
    seed_shared_value(&store, &session_pin("s1"), RouteValue::Counter(1), 600).await;
    let fixture = Fixture::with_shared(settings(&[AffinityFlag::Session]), &[], store);
    let healthy = pool(&["a", "b"]);
    let kwargs = metadata(&[("session_id", text("s1"))]);
    assert_eq!(fixture.filter(&healthy, Some(&kwargs)).await, ["a", "b"]);
}

// a bare string pin left by an older writer still selects its deployment
pub(super) async fn legacy_text_pin_selects() {
    let fixture = Fixture::new(settings(&[AffinityFlag::Deployment]), &[]);
    fixture.seed(&user_pin("key"), &StoredPin::Text("b".to_owned()), 600);
    let healthy = pool(&["a", "b"]);
    let kwargs = metadata(&[("user_api_key_hash", text("key"))]);
    assert_eq!(fixture.filter(&healthy, Some(&kwargs)).await, ["b"]);
}

// a non-pin value older writers left behind selects nothing
pub(super) async fn other_pin_ignored() {
    let fixture = Fixture::new(settings(&[AffinityFlag::Deployment]), &[]);
    fixture.seed(&user_pin("key"), &StoredPin::Other(Json::Bool(true)), 600);
    let healthy = pool(&["a", "b"]);
    let kwargs = metadata(&[("user_api_key_hash", text("key"))]);
    assert_eq!(fixture.filter(&healthy, Some(&kwargs)).await, ["a", "b"]);
}

// a pin whose model id is empty selects nothing
pub(super) async fn empty_model_id_pin_ignored() {
    let fixture = Fixture::new(settings(&[AffinityFlag::Deployment]), &[]);
    fixture.seed(&user_pin("key"), &pin(""), 600);
    let healthy = pool(&["a", "b"]);
    let kwargs = metadata(&[("user_api_key_hash", text("key"))]);
    assert_eq!(fixture.filter(&healthy, Some(&kwargs)).await, ["a", "b"]);
}

// with the session flag off, the marker alone lets the hook pin the session
pub(super) async fn hook_session_only_with_marker() {
    let fixture = Fixture::new(settings(&[]), &[]);
    let hook_kwargs = hook_request(
        &[
            ("deployment_model_name", text(MODEL_GROUP)),
            ("session_id", text("s1")),
            ("_session_deployment_affinity_ttl", number(30)),
        ],
        Some("a"),
    );
    fixture.hook(&hook_kwargs, None).await.unwrap();
    let healthy = pool(&["a", "b"]);
    let kwargs = metadata(&[("session_id", text("s1"))]);
    assert_eq!(fixture.filter(&healthy, Some(&kwargs)).await, ["a"]);
}

// the marker's ttl, not the router ttl, bounds the session pin
pub(super) async fn hook_marker_sets_session_ttl() {
    let fixture = Fixture::new(settings_with_ttl(&[], 3600), &[]);
    let hook_kwargs = hook_request(
        &[
            ("deployment_model_name", text(MODEL_GROUP)),
            ("session_id", text("s1")),
            ("_session_deployment_affinity_ttl", number(5)),
        ],
        Some("a"),
    );
    fixture.hook(&hook_kwargs, None).await.unwrap();
    assert_eq!(fixture.expiry(&session_pin("s1")), Some(5.0));
}

// the router ttl bounds the caller pin
pub(super) async fn hook_user_ttl_uses_settings() {
    let fixture = Fixture::new(settings_with_ttl(&[AffinityFlag::Deployment], 120), &[]);
    let hook_kwargs = hook_request(
        &[
            ("deployment_model_name", text(MODEL_GROUP)),
            ("user_api_key_hash", text("key")),
        ],
        Some("a"),
    );
    fixture.hook(&hook_kwargs, None).await.unwrap();
    assert_eq!(fixture.expiry(&user_pin("key")), Some(120.0));
}

// without any model_info the hook writes no pin
pub(super) async fn hook_without_model_info() {
    let fixture = Fixture::new(settings(&[AffinityFlag::Deployment]), &[]);
    let hook_kwargs = hook_request(
        &[
            ("deployment_model_name", text(MODEL_GROUP)),
            ("user_api_key_hash", text("key")),
        ],
        None,
    );
    fixture.hook(&hook_kwargs, None).await.unwrap();
    let healthy = pool(&["a", "b"]);
    let kwargs = metadata(&[("user_api_key_hash", text("key"))]);
    assert_eq!(fixture.filter(&healthy, Some(&kwargs)).await, ["a", "b"]);
}

// an empty or non-text model id warns and writes no pin
pub(super) async fn hook_unusable_model_id() {
    let fixture = Fixture::new(settings(&[AffinityFlag::Deployment]), &[]);
    let empty = model_info("");
    let kwargs = hook_request(
        &[
            ("deployment_model_name", text(MODEL_GROUP)),
            ("user_api_key_hash", text("key")),
        ],
        None,
    );
    fixture.hook(&kwargs, Some(&empty)).await.unwrap();
    assert_eq!(fixture.expiry(&user_pin("key")), None);
    let mut numeric = model_info("");
    numeric.insert("id".to_owned(), number(7));
    fixture.hook(&kwargs, Some(&numeric)).await.unwrap();
    let healthy = pool(&["a", "b"]);
    let filter_kwargs = metadata(&[("user_api_key_hash", text("key"))]);
    assert_eq!(
        fixture.filter(&healthy, Some(&filter_kwargs)).await,
        ["a", "b"]
    );
    assert_eq!(fixture.expiry(&user_pin("key")), None);
}
