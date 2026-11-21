use betterlite_models::decisions::Json;

use super::support::{
    Fixture, MODEL_GROUP, deployment, encoded_previous_response_id, groups, hook_request,
    litellm_metadata, metadata, number, pin, pool, session_pin, settings, settings_with_ttl, text,
    user_pin, with_groups,
};
use crate::affinity_claims::identity::AffinityFlag;
use crate::deployment_affinity::flags::warn_on_unknown_model_group_affinity_flags;

// a pool with no pin keeps every healthy deployment
pub(super) async fn no_pin_keeps_pool() {
    let fixture = Fixture::new(settings(&[AffinityFlag::Deployment]), &[]);
    let healthy = pool(&["a", "b"]);
    let kwargs = metadata(&[("user_api_key_hash", text("key"))]);
    assert_eq!(fixture.filter(&healthy, Some(&kwargs)).await, ["a", "b"]);
}

// the caller key pin narrows the pool to its deployment
pub(super) async fn caller_pin_selects() {
    let fixture = Fixture::new(settings(&[AffinityFlag::Deployment]), &[]);
    fixture.seed(&user_pin("key"), &pin("a"), 600);
    let healthy = pool(&["a", "b"]);
    let kwargs = metadata(&[("user_api_key_hash", text("key"))]);
    assert_eq!(fixture.filter(&healthy, Some(&kwargs)).await, ["a"]);
}

// an active session pin selects its deployment
pub(super) async fn session_pin_selects() {
    let fixture = Fixture::new(settings(&[AffinityFlag::Session]), &[]);
    fixture.seed(&session_pin("s1"), &pin("b"), 600);
    let healthy = pool(&["a", "b"]);
    let kwargs = metadata(&[("session_id", text("s1"))]);
    assert_eq!(fixture.filter(&healthy, Some(&kwargs)).await, ["b"]);
}

// the session pin is read before the caller pin
pub(super) async fn session_beats_caller() {
    let fixture = Fixture::new(
        settings(&[AffinityFlag::Deployment, AffinityFlag::Session]),
        &[],
    );
    fixture.seed(&session_pin("s1"), &pin("a"), 600);
    fixture.seed(&user_pin("key"), &pin("b"), 600);
    let healthy = pool(&["a", "b"]);
    let kwargs = metadata(&[
        ("user_api_key_hash", text("key")),
        ("session_id", text("s1")),
    ]);
    assert_eq!(fixture.filter(&healthy, Some(&kwargs)).await, ["a"]);
}

// the marker turns session pinning on for a group whose global session flag is off
pub(super) async fn marker_enables_session() {
    let fixture = Fixture::new(settings(&[AffinityFlag::Deployment]), &[]);
    fixture.seed(&session_pin("s1"), &pin("a"), 600);
    let healthy = pool(&["a", "b"]);
    let kwargs = metadata(&[
        ("session_id", text("s1")),
        ("_session_deployment_affinity_ttl", number(30)),
    ]);
    assert_eq!(fixture.filter(&healthy, Some(&kwargs)).await, ["a"]);
}

// previous_response_id continuity wins over an active session pin
pub(super) async fn responses_beats_session() {
    let fixture = Fixture::new(
        settings(&[AffinityFlag::ResponsesApi, AffinityFlag::Session]),
        &[],
    );
    fixture.seed(&session_pin("s1"), &pin("a"), 600);
    let healthy = pool(&["a", "b"]);
    let kwargs = super::support::request(&[
        (
            "metadata",
            Json::Object(super::support::request(&[("session_id", text("s1"))])),
        ),
        (
            "previous_response_id",
            text(&encoded_previous_response_id("b")),
        ),
    ]);
    assert_eq!(fixture.filter(&healthy, Some(&kwargs)).await, ["b"]);
}

// a priority routing group leaves the pool to the strategy
pub(super) async fn priority_group_keeps_pool() {
    let fixture = Fixture::new(settings(&[AffinityFlag::Deployment]), &[MODEL_GROUP]);
    fixture.seed(&user_pin("key"), &pin("a"), 600);
    let healthy = pool(&["a", "b"]);
    let kwargs = metadata(&[("user_api_key_hash", text("key"))]);
    assert_eq!(fixture.filter(&healthy, Some(&kwargs)).await, ["a", "b"]);
}

// a fallback retry carries _target_order and stands the check down
pub(super) async fn target_order_keeps_pool() {
    let fixture = Fixture::new(settings(&[AffinityFlag::Deployment]), &[]);
    fixture.seed(&user_pin("key"), &pin("a"), 600);
    let healthy = pool(&["a", "b"]);
    let kwargs = metadata(&[("user_api_key_hash", text("key"))]);
    assert_eq!(
        fixture
            .filter_at(MODEL_GROUP, &healthy, Some(&kwargs), Some(2))
            .await,
        ["a", "b"]
    );
}

// a per-group entry enables the flag the global settings leave off
pub(super) async fn group_flags_enable() {
    let fixture = Fixture::new(
        with_groups(settings(&[]), &[(MODEL_GROUP, &["deployment_affinity"])]),
        &[],
    );
    fixture.seed(&user_pin("key"), &pin("a"), 600);
    let healthy = pool(&["a", "b"]);
    let kwargs = metadata(&[("user_api_key_hash", text("key"))]);
    assert_eq!(fixture.filter(&healthy, Some(&kwargs)).await, ["a"]);
}

// an entry with no flags disables the group even when the globals enable it
pub(super) async fn group_entry_disables() {
    let fixture = Fixture::new(
        with_groups(settings(&[AffinityFlag::Deployment]), &[(MODEL_GROUP, &[])]),
        &[],
    );
    fixture.seed(&user_pin("key"), &pin("a"), 600);
    let healthy = pool(&["a", "b"]);
    let kwargs = metadata(&[("user_api_key_hash", text("key"))]);
    assert_eq!(fixture.filter(&healthy, Some(&kwargs)).await, ["a", "b"]);
}

// a group without an entry falls back to the global flags
pub(super) async fn group_without_entry_uses_globals() {
    let fixture = Fixture::new(
        with_groups(settings(&[AffinityFlag::Deployment]), &[("other", &[])]),
        &[],
    );
    fixture.seed(&user_pin("key"), &pin("a"), 600);
    let healthy = pool(&["a", "b"]);
    let kwargs = metadata(&[("user_api_key_hash", text("key"))]);
    assert_eq!(fixture.filter(&healthy, Some(&kwargs)).await, ["a"]);
}

// a pool whose deployments disagree on the model map key is never pinned
pub(super) async fn unstable_group_keeps_pool() {
    let fixture = Fixture::new(settings(&[AffinityFlag::Deployment]), &[]);
    fixture.seed(&user_pin("key"), &pin("a"), 600);
    let healthy = vec![deployment(MODEL_GROUP, "a"), deployment("other", "b")];
    let kwargs = metadata(&[("user_api_key_hash", text("key"))]);
    assert_eq!(fixture.filter(&healthy, Some(&kwargs)).await, ["a", "b"]);
}

// a pin whose deployment left the healthy pool is not brought back
pub(super) async fn filtered_pin_not_revived() {
    let fixture = Fixture::new(settings(&[AffinityFlag::Deployment]), &[]);
    fixture.seed(&user_pin("key"), &pin("a"), 600);
    let healthy = pool(&["b"]);
    let kwargs = metadata(&[("user_api_key_hash", text("key"))]);
    assert_eq!(fixture.filter(&healthy, Some(&kwargs)).await, ["b"]);
}

// the pre-call hook persists the selected deployment for the caller
pub(super) async fn hook_pins_caller() {
    let fixture = Fixture::new(settings(&[AffinityFlag::Deployment]), &[]);
    let kwargs = hook_request(
        &[
            ("deployment_model_name", text(MODEL_GROUP)),
            ("user_api_key_hash", text("key")),
        ],
        Some("a"),
    );
    fixture.hook(&kwargs, None).await.unwrap();
    let healthy = pool(&["a", "b"]);
    assert_eq!(fixture.filter(&healthy, Some(&kwargs)).await, ["a"]);
}

// the pre-call hook persists the selected deployment for the session
pub(super) async fn hook_pins_session() {
    let fixture = Fixture::new(settings(&[AffinityFlag::Session]), &[]);
    let hook_kwargs = hook_request(
        &[
            ("deployment_model_name", text(MODEL_GROUP)),
            ("session_id", text("s1")),
        ],
        Some("b"),
    );
    fixture.hook(&hook_kwargs, None).await.unwrap();
    let healthy = pool(&["a", "b"]);
    let kwargs = metadata(&[("session_id", text("s1"))]);
    assert_eq!(fixture.filter(&healthy, Some(&kwargs)).await, ["b"]);
}

// without deployment_model_name the hook cannot resolve the group's flags, so it writes nothing
pub(super) async fn hook_needs_deployment_model_name() {
    let fixture = Fixture::new(settings(&[AffinityFlag::Deployment]), &[]);
    let kwargs = hook_request(&[("user_api_key_hash", text("key"))], Some("a"));
    fixture.hook(&kwargs, None).await.unwrap();
    let healthy = pool(&["a", "b"]);
    assert_eq!(fixture.filter(&healthy, Some(&kwargs)).await, ["a", "b"]);
}

// with every flag off the hook writes nothing
pub(super) async fn hook_needs_a_flag() {
    let fixture = Fixture::new(settings(&[]), &[]);
    let kwargs = hook_request(
        &[
            ("deployment_model_name", text(MODEL_GROUP)),
            ("user_api_key_hash", text("key")),
        ],
        Some("a"),
    );
    fixture.hook(&kwargs, None).await.unwrap();
    let healthy = pool(&["a", "b"]);
    assert_eq!(fixture.filter(&healthy, Some(&kwargs)).await, ["a", "b"]);
}

// an existing pin wins over a later claim for the same caller
pub(super) async fn hook_first_writer_wins() {
    let fixture = Fixture::new(settings(&[AffinityFlag::Deployment]), &[]);
    fixture.seed(&user_pin("key"), &pin("a"), 600);
    let kwargs = hook_request(
        &[
            ("deployment_model_name", text(MODEL_GROUP)),
            ("user_api_key_hash", text("key")),
        ],
        Some("b"),
    );
    fixture.hook(&kwargs, None).await.unwrap();
    let healthy = pool(&["a", "b"]);
    assert_eq!(fixture.filter(&healthy, Some(&kwargs)).await, ["a"]);
}

// an expired pin no longer wins, so the next claim replaces it
pub(super) async fn expired_pin_is_replaced() {
    let fixture = Fixture::new(settings_with_ttl(&[AffinityFlag::Deployment], 10), &[]);
    fixture.seed(&user_pin("key"), &pin("a"), 10);
    fixture.advance(20.0);
    let kwargs = hook_request(
        &[
            ("deployment_model_name", text(MODEL_GROUP)),
            ("user_api_key_hash", text("key")),
        ],
        Some("b"),
    );
    fixture.hook(&kwargs, None).await.unwrap();
    let healthy = pool(&["a", "b"]);
    assert_eq!(fixture.filter(&healthy, Some(&kwargs)).await, ["b"]);
}

// the hook reads the model_info metadata channel when the request carries none
pub(super) async fn hook_reads_metadata_model_info() {
    let fixture = Fixture::new(settings(&[AffinityFlag::Deployment]), &[]);
    let kwargs = super::support::request(&[(
        "litellm_metadata",
        Json::Object(super::support::request(&[
            ("deployment_model_name", text(MODEL_GROUP)),
            ("user_api_key_hash", text("key")),
            ("model_info", Json::Object(super::support::model_info("a"))),
        ])),
    )]);
    fixture.hook(&kwargs, None).await.unwrap();
    let healthy = pool(&["a", "b"]);
    assert_eq!(fixture.filter(&healthy, Some(&kwargs)).await, ["a"]);
}

// warn_on_unknown_model_group_affinity_flags names each group's unknown flags once, sorted
pub(super) fn unknown_flags_warn() {
    let config = groups(&[(
        MODEL_GROUP,
        &["deployment_affinity", "wrong", "session_affinity", "typo"],
    )]);
    let warnings = warn_on_unknown_model_group_affinity_flags(Some(&config));
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].group, MODEL_GROUP);
    assert_eq!(warnings[0].unknown, ["typo", "wrong"]);
    assert_eq!(
        warnings[0].message(),
        "model_group_affinity_config: unknown flag(s) 'typo', 'wrong' for model group 'gpt-4o'; will be ignored. Valid flags: frozenset({'deployment_affinity', 'encrypted_content_affinity', 'responses_api_deployment_check', 'session_affinity'})"
    );
    assert!(warn_on_unknown_model_group_affinity_flags(None).is_empty());
    let known = groups(&[(MODEL_GROUP, &["encrypted_content_affinity"])]);
    assert!(warn_on_unknown_model_group_affinity_flags(Some(&known)).is_empty());
}

// the litellm_metadata channel is read first for the caller key
pub(super) async fn litellm_metadata_caller_key() {
    let fixture = Fixture::new(settings(&[AffinityFlag::Deployment]), &[]);
    fixture.seed(&user_pin("key"), &pin("a"), 600);
    let healthy = pool(&["a", "b"]);
    let kwargs = litellm_metadata(&[("user_api_key_hash", text("key"))]);
    assert_eq!(fixture.filter(&healthy, Some(&kwargs)).await, ["a"]);
}
