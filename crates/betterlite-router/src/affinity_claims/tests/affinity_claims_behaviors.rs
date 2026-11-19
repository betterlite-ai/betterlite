use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use serde_json::Number;

use super::support::{
    TestClock, deployment, flags, key, local_pins, metadata, pin, runtime, shared_store,
};
use crate::affinity_claims::AffinityClaims;
use crate::affinity_claims::identity::{
    affinity_cache_key, effective_flags, find_deployment_by_model_id, hash_user_key,
    marker_session_affinity_ttl, model_map_key_from_deployment, model_map_key_from_model_name,
    session_affinity_cache_key, session_id_from_request_kwargs, shorten_for_logs,
    stable_model_map_key, user_key_from_request_kwargs,
};
use crate::affinity_claims::pin::StoredPin;
use crate::affinity_claims::shared::SharedFallback;
use crate::routing_state_store::test_support::FakeRedis;

#[test]
fn a_session_key_scopes_the_same_session_id_by_caller() {
    let first = session_affinity_cache_key("group-a", "session-1", Some("caller-one")).unwrap();
    let second = session_affinity_cache_key("group-a", "session-1", Some("caller-two")).unwrap();
    let unscoped = session_affinity_cache_key("group-a", "session-1", None).unwrap();
    assert_ne!(first, second);
    assert_ne!(first, unscoped);
    let hashed = hash_user_key("caller-one");
    assert_eq!(
        first.as_ref(),
        format!("deployment_affinity:v1:session:group-a:{hashed}:session-1")
    );
    assert_eq!(
        unscoped.as_ref(),
        "deployment_affinity:v1:session:group-a:unscoped:session-1"
    );
}

#[test]
fn an_already_hashed_caller_key_is_kept_and_lowercased() {
    let digest = "A".repeat(64);
    let built = affinity_cache_key("group-a", &digest).unwrap();
    assert_eq!(
        built.as_ref(),
        format!("deployment_affinity:v1:group-a:{}", digest.to_lowercase())
    );
}

#[test]
fn the_api_key_hash_wins_over_a_user_id_from_the_other_metadata_channel() {
    let mut kwargs = JsonObject::new();
    kwargs.insert(
        "litellm_metadata".to_owned(),
        Json::Object(metadata(&[(
            "user_api_key_user_id",
            Json::Text("user-9".to_owned()),
        )])),
    );
    kwargs.insert(
        "metadata".to_owned(),
        Json::Object(metadata(&[(
            "user_api_key_hash",
            Json::Text("hash-9".to_owned()),
        )])),
    );
    assert_eq!(
        user_key_from_request_kwargs(&kwargs).unwrap().as_deref(),
        Some("hash-9")
    );
}

#[test]
fn a_user_id_is_namespaced_and_no_metadata_names_no_caller() {
    let mut kwargs = JsonObject::new();
    kwargs.insert(
        "metadata".to_owned(),
        Json::Object(metadata(&[(
            "user_api_key_user_id",
            Json::Text("user-9".to_owned()),
        )])),
    );
    assert_eq!(
        user_key_from_request_kwargs(&kwargs).unwrap().as_deref(),
        Some("user_id:user-9")
    );
    assert_eq!(
        user_key_from_request_kwargs(&JsonObject::new()).unwrap(),
        None
    );
}

#[test]
fn a_generated_session_marker_hides_only_its_own_channel() {
    let generated = metadata(&[
        ("session_id", Json::Text("minted".to_owned())),
        ("litellm_session_id_generated", Json::Bool(true)),
    ]);
    let caller = metadata(&[("session_id", Json::Text("caller".to_owned()))]);
    let mut kwargs = JsonObject::new();
    kwargs.insert(
        "litellm_metadata".to_owned(),
        Json::Object(generated.clone()),
    );
    kwargs.insert("metadata".to_owned(), Json::Object(caller));
    assert_eq!(
        session_id_from_request_kwargs(&kwargs).unwrap().as_deref(),
        Some("caller")
    );
    let mut only_generated = JsonObject::new();
    only_generated.insert("litellm_metadata".to_owned(), Json::Object(generated));
    assert_eq!(
        session_id_from_request_kwargs(&only_generated).unwrap(),
        None
    );
}

#[test]
fn the_session_marker_ttl_takes_the_first_positive_integer() {
    let mut kwargs = JsonObject::new();
    kwargs.insert(
        "litellm_metadata".to_owned(),
        Json::Object(metadata(&[(
            "_session_deployment_affinity_ttl",
            Json::Number(Number::from(0_u8)),
        )])),
    );
    kwargs.insert(
        "metadata".to_owned(),
        Json::Object(metadata(&[(
            "_session_deployment_affinity_ttl",
            Json::Number(Number::from(45_u8)),
        )])),
    );
    assert_eq!(marker_session_affinity_ttl(&kwargs).unwrap(), Some(45));
}

#[test]
fn an_explicit_group_entry_replaces_the_instance_flags() {
    let mut config = indexmap::IndexMap::new();
    config.insert("group-a".to_owned(), vec!["session_affinity".to_owned()]);
    let defaults = flags(&["deployment_affinity"]);
    let selected = effective_flags(Some(&config), "group-a", &defaults);
    assert_eq!(selected, flags(&["session_affinity"]));
    let fallback = effective_flags(Some(&config), "group-b", &defaults);
    assert_eq!(fallback, defaults);
    let empty = effective_flags(Some(&config), "group-a", &flags(&[]));
    assert_ne!(empty, flags(&[]));
    config.insert("group-c".to_owned(), vec!["unknown_flag".to_owned()]);
    assert!(effective_flags(Some(&config), "group-c", &defaults).is_empty());
}

#[test]
fn the_model_map_key_strips_the_provider_but_never_an_azure_deployment() {
    assert_eq!(
        model_map_key_from_model_name("openai/gpt-4o"),
        Some("gpt-4o".to_owned())
    );
    assert_eq!(model_map_key_from_model_name("azure/deployment-a"), None);
    assert_eq!(
        model_map_key_from_model_name("gpt-4o"),
        Some("gpt-4o".to_owned())
    );
    assert_eq!(model_map_key_from_model_name(""), None);
}

#[test]
fn a_deployment_map_key_prefers_the_group_then_the_base_models() {
    let named = deployment("group-a", &[("model", "openai/gpt-4o")], "dep-a");
    assert_eq!(
        model_map_key_from_deployment(&named),
        Some("group-a".to_owned())
    );
    let info_base = deployment("", &[], "dep-b");
    assert_eq!(model_map_key_from_deployment(&info_base), None);
    let params_base = deployment("", &[("base_model", "gpt-4o")], "dep-c");
    assert_eq!(
        model_map_key_from_deployment(&params_base),
        Some("gpt-4o".to_owned())
    );
    let bare = deployment("", &[("model", "openai/gpt-4o")], "dep-d");
    assert_eq!(
        model_map_key_from_deployment(&bare),
        Some("gpt-4o".to_owned())
    );
    let azure = deployment("", &[("model", "azure/deployment-name")], "dep-e");
    assert_eq!(model_map_key_from_deployment(&azure), None);
}

#[test]
fn a_stable_model_map_key_needs_one_key_across_the_deployments() {
    let first = deployment("group-a", &[], "dep-a");
    let same = deployment("", &[("base_model", "group-a")], "dep-b");
    assert_eq!(
        stable_model_map_key(&[first, same]),
        Some("group-a".to_owned())
    );
    let other = deployment("group-b", &[], "dep-c");
    let mixed = deployment("", &[("model", "openai/gpt-4o")], "dep-d");
    assert_eq!(stable_model_map_key(&[other, mixed]), None);
    assert_eq!(stable_model_map_key(&[]), None);
}

#[test]
fn a_pin_model_id_comes_from_a_pin_or_a_legacy_string() {
    assert_eq!(
        StoredPin::Pin(pin("model_id", "dep-a"))
            .pinned_model_id()
            .as_deref(),
        Some("dep-a")
    );
    assert_eq!(
        StoredPin::Text("dep-b".to_owned())
            .pinned_model_id()
            .as_deref(),
        Some("dep-b")
    );
    assert_eq!(
        StoredPin::Pin(pin("model", "dep-c")).pinned_model_id(),
        None
    );
    assert_eq!(
        StoredPin::Other(Json::List(Vec::new())).pinned_model_id(),
        None
    );
}

#[test]
fn log_values_keep_eight_characters_then_an_ellipsis() {
    assert_eq!(shorten_for_logs("abcdefgh"), "abcdefgh");
    assert_eq!(shorten_for_logs("abcdefghi"), "abcdefgh...");
    assert_eq!(shorten_for_logs("ééééééééé"), "éééééééé...");
}

#[test]
fn a_deployment_is_found_by_its_model_info_id() {
    let deployments = [
        deployment("group-a", &[], "dep-a"),
        deployment("group-a", &[], "dep-b"),
    ];
    let found = find_deployment_by_model_id(&deployments, "dep-b").unwrap();
    assert_eq!(found.id(), Some("dep-b"));
    assert_eq!(find_deployment_by_model_id(&deployments, "dep-c"), None);
}

#[test]
fn the_first_local_writer_keeps_its_pin() {
    let clock = TestClock::default();
    clock.at(1_700_000_000.0);
    let pins = local_pins(&clock);
    let first = pins
        .claim_in_memory(
            &key("deployment_affinity:v1:group:caller"),
            &pin("model_id", "dep-a"),
            60,
            None,
        )
        .unwrap();
    let second = pins
        .claim_in_memory(
            &key("deployment_affinity:v1:group:caller"),
            &pin("model_id", "dep-b"),
            60,
            None,
        )
        .unwrap();
    assert_eq!(first, StoredPin::Pin(pin("model_id", "dep-a")));
    assert_eq!(second, first);
}

#[test]
fn a_matching_legacy_local_pin_is_rewritten_and_a_different_one_is_kept() {
    let clock = TestClock::default();
    clock.at(1_700_000_000.0);
    let pins = local_pins(&clock);
    let matching = key("deployment_affinity:v1:group:matching");
    pins.set(&matching, &StoredPin::Text("dep-a".to_owned()), 60)
        .unwrap();
    let winner = pins
        .claim_in_memory(&matching, &pin("model_id", "dep-a"), 60, None)
        .unwrap();
    assert_eq!(winner, StoredPin::Text("dep-a".to_owned()));
    assert_eq!(
        pins.read(&matching).unwrap(),
        Some(StoredPin::Pin(pin("model_id", "dep-a")))
    );
    let different = key("deployment_affinity:v1:group:different");
    pins.set(&different, &StoredPin::Text("dep-x".to_owned()), 60)
        .unwrap();
    let kept = pins
        .claim_in_memory(&different, &pin("model_id", "dep-a"), 60, None)
        .unwrap();
    assert_eq!(kept, StoredPin::Text("dep-x".to_owned()));
    assert_eq!(
        pins.read(&different).unwrap(),
        Some(StoredPin::Text("dep-x".to_owned()))
    );
}

#[test]
fn an_eligible_local_claim_keeps_a_matching_winner_and_replaces_a_stale_one() {
    let clock = TestClock::default();
    clock.at(1_700_000_000.0);
    let pins = local_pins(&clock);
    let cache_key = key("deployment_affinity:v1:group:caller");
    pins.set(&cache_key, &StoredPin::Pin(pin("model_id", "dep-a")), 60)
        .unwrap();
    let candidates = [pin("model_id", "dep-a"), pin("model_id", "dep-b")];
    let kept = pins
        .claim_in_memory(&cache_key, &pin("model_id", "dep-b"), 60, Some(&candidates))
        .unwrap();
    assert_eq!(kept, StoredPin::Pin(pin("model_id", "dep-a")));
    let only = [pin("model_id", "dep-c")];
    let replaced = pins
        .claim_in_memory(&cache_key, &pin("model_id", "dep-c"), 60, Some(&only))
        .unwrap();
    assert_eq!(replaced, StoredPin::Pin(pin("model_id", "dep-c")));
}

#[test]
fn a_repeated_claim_refreshes_the_local_expiry() {
    let clock = TestClock::default();
    clock.at(1_700_000_000.0);
    let pins = local_pins(&clock);
    let cache_key = key("deployment_affinity:v1:group:caller");
    pins.claim_in_memory(&cache_key, &pin("model_id", "dep-a"), 60, None)
        .unwrap();
    clock.at(1_700_000_050.0);
    pins.claim_in_memory(&cache_key, &pin("model_id", "dep-a"), 60, None)
        .unwrap();
    clock.at(1_700_000_070.0);
    assert_eq!(
        pins.read(&cache_key).unwrap(),
        Some(StoredPin::Pin(pin("model_id", "dep-a")))
    );
    clock.at(1_700_000_111.0);
    assert_eq!(pins.read(&cache_key).unwrap(), None);
}

#[test]
fn the_shared_first_writer_wins_across_workers_and_mirrors_locally() {
    runtime().block_on(async {
        let server = FakeRedis::try_start().await.unwrap();
        let clock = TestClock::default();
        clock.at(1_700_000_000.0);
        let first = AffinityClaims {
            local: local_pins(&clock),
            shared: Some(shared_store(&server)),
        };
        let second = AffinityClaims {
            local: local_pins(&clock),
            shared: Some(shared_store(&server)),
        };
        let cache_key = key("deployment_affinity:v1:group:caller");
        let claimed = first
            .claim(&cache_key, &pin("model_id", "dep-a"), 60, None)
            .await
            .unwrap();
        assert_eq!(claimed.pin, StoredPin::Pin(pin("model_id", "dep-a")));
        assert_eq!(claimed.fallback, None);
        let from_second = second
            .claim(&cache_key, &pin("model_id", "dep-b"), 60, None)
            .await
            .unwrap();
        assert_eq!(from_second.pin, StoredPin::Pin(pin("model_id", "dep-a")));
        assert_eq!(from_second.fallback, None);
        assert_eq!(
            second.local.read(&cache_key).unwrap(),
            Some(StoredPin::Pin(pin("model_id", "dep-a")))
        );
    });
}

#[test]
fn a_shared_eligible_claim_keeps_its_winner_and_refreshes_the_shared_ttl() {
    runtime().block_on(async {
        let server = FakeRedis::try_start().await.unwrap();
        let clock = TestClock::default();
        clock.at(1_700_000_000.0);
        let service = AffinityClaims {
            local: local_pins(&clock),
            shared: Some(shared_store(&server)),
        };
        let cache_key = key("deployment_affinity:v1:group:caller");
        service
            .claim(&cache_key, &pin("model_id", "dep-a"), 30, None)
            .await
            .unwrap();
        let candidates = [pin("model_id", "dep-a"), pin("model_id", "dep-b")];
        let kept = service
            .claim(&cache_key, &pin("model_id", "dep-b"), 60, Some(&candidates))
            .await
            .unwrap();
        assert_eq!(kept.pin, StoredPin::Pin(pin("model_id", "dep-a")));
        let ttl = service
            .shared
            .as_ref()
            .unwrap()
            .get_ttl(&cache_key.0)
            .await
            .unwrap()
            .value;
        assert_eq!(ttl, Some(60));
        let only = [pin("model_id", "dep-c")];
        let replaced = service
            .claim(&cache_key, &pin("model_id", "dep-c"), 60, Some(&only))
            .await
            .unwrap();
        assert_eq!(replaced.pin, StoredPin::Pin(pin("model_id", "dep-c")));
        let raw = server.raw(cache_key.as_ref()).unwrap().0;
        let text = String::from_utf8(raw).unwrap();
        assert!(text.contains("dep-c"));
    });
}

#[test]
fn a_redis_outage_falls_back_to_the_local_claim_and_recovers() {
    runtime().block_on(async {
        let server = FakeRedis::try_start().await.unwrap();
        let clock = TestClock::default();
        clock.at(1_700_000_000.0);
        let service = AffinityClaims {
            local: local_pins(&clock),
            shared: Some(shared_store(&server)),
        };
        let cache_key = key("deployment_affinity:v1:group:caller");
        service
            .claim(&cache_key, &pin("model_id", "dep-a"), 60, None)
            .await
            .unwrap();
        server.set_down(true);
        let fallback = service
            .claim(&cache_key, &pin("model_id", "dep-b"), 60, None)
            .await
            .unwrap();
        assert_eq!(fallback.pin, StoredPin::Pin(pin("model_id", "dep-a")));
        assert_eq!(fallback.fallback, Some(SharedFallback::Unavailable));
        server.set_down(false);
        let recovered = service
            .claim(&cache_key, &pin("model_id", "dep-c"), 60, None)
            .await
            .unwrap();
        assert_eq!(recovered.pin, StoredPin::Pin(pin("model_id", "dep-a")));
        assert_eq!(recovered.fallback, None);
    });
}
