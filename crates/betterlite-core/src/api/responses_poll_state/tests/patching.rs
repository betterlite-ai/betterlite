use indexmap::IndexMap;

use super::super::access::PollAccess;
use super::super::error::UnknownPatchField;
use super::super::get_cache_key;
use super::super::record::PollPatch;
use super::super::store::{PollingUpdate, ResponsePollingStore};
use super::support::{self, CacheOperation, FakeCache, Watch};

const ID: &str = "litellm_poll_patch";
pub(super) const TTL: i64 = 17;
const SEED: &str = concat!(
    r#"{"id":"litellm_poll_patch","object":"response","status":"queued","#,
    r#""created_at":1700000000,"output":[{"id":"old"}],"#,
    r#""usage":{"total_tokens":1},"error":{"code":"old"}}"#
);
const PRESENT: &str = concat!(
    r#"{"reasoning":{},"tool_choice":"","tools":[],"model":"","#,
    r#""instructions":"","temperature":0,"top_p":0,"max_output_tokens":0,"#,
    r#""previous_response_id":"","text":{},"truncation":"","#,
    r#""parallel_tool_calls":false,"user":"","store":false}"#
);

pub(super) async fn falsy_patches(cache: &FakeCache, store: &ResponsePollingStore) {
    assert_empty_status_ignored(cache, store).await;
    assert_empty_output_replaces(cache, store).await;
    assert_empty_objects_ignored(cache, store).await;
}

async fn assert_empty_status_ignored(cache: &FakeCache, store: &ResponsePollingStore) {
    let update = store
        .update_state(
            ID,
            PollAccess::AuthDisabled,
            &support::patch(r#"{"status":""}"#),
        )
        .await
        .expect("the empty status is ignored");
    assert_eq!(update, PollingUpdate::Updated);
    let written = support::stored(cache, ID);
    assert_eq!(
        support::field(&written, "status"),
        support::json("\"queued\"")
    );
    assert_eq!(
        support::field(&written, "output"),
        support::json(r#"[{"id":"old"}]"#)
    );
}

async fn assert_empty_output_replaces(cache: &FakeCache, store: &ResponsePollingStore) {
    let _update = store
        .update_state(
            ID,
            PollAccess::AuthDisabled,
            &support::patch(r#"{"output":[]}"#),
        )
        .await
        .expect("an empty output list replaces the old one");
    let written = support::stored(cache, ID);
    assert_eq!(support::field(&written, "output"), support::json("[]"));
}

async fn assert_empty_objects_ignored(cache: &FakeCache, store: &ResponsePollingStore) {
    let _update = store
        .update_state(
            ID,
            PollAccess::AuthDisabled,
            &support::patch(r#"{"usage":{},"error":{},"incomplete_details":{}}"#),
        )
        .await
        .expect("the empty truthy fields leave the state alone");
    let written = support::stored(cache, ID);
    assert_eq!(
        support::field(&written, "usage"),
        support::json(r#"{"total_tokens":1}"#)
    );
    assert_eq!(
        support::field(&written, "error"),
        support::json(r#"{"code":"old"}"#)
    );
    assert!(!support::has_key(&written, "incomplete_details"));
}

pub(super) async fn error_overrides_status(cache: &FakeCache, store: &ResponsePollingStore) {
    let _update = store
        .update_state(
            ID,
            PollAccess::AuthDisabled,
            &support::patch(r#"{"status":"completed","error":{"code":"bad","message":"failed"}}"#),
        )
        .await
        .expect("the failed state is written");
    let written = support::stored(cache, ID);
    assert_eq!(
        support::field(&written, "status"),
        support::json("\"failed\"")
    );
    assert_eq!(
        support::field(&written, "error"),
        support::json(r#"{"code":"bad","message":"failed"}"#)
    );
}

pub(super) async fn present_fields(cache: &FakeCache, store: &ResponsePollingStore) {
    let _update = store
        .update_state(ID, PollAccess::AuthDisabled, &support::patch(PRESENT))
        .await
        .expect("every non-null field replaces its value");
    let written = support::stored(cache, ID);
    let expected = [
        ("reasoning", "{}"),
        ("tool_choice", "\"\""),
        ("tools", "[]"),
        ("model", "\"\""),
        ("instructions", "\"\""),
        ("temperature", "0"),
        ("top_p", "0"),
        ("max_output_tokens", "0"),
        ("previous_response_id", "\"\""),
        ("text", "{}"),
        ("truncation", "\"\""),
        ("parallel_tool_calls", "false"),
        ("user", "\"\""),
        ("store", "false"),
    ];
    for (key, value) in expected {
        assert_eq!(support::field(&written, key), support::json(value), "{key}");
    }
}

pub(super) async fn null_patch_leaves_fields(cache: &FakeCache, store: &ResponsePollingStore) {
    let before = support::stored(cache, ID);
    let _update = store
        .update_state(
            ID,
            PollAccess::AuthDisabled,
            &support::patch(concat!(
                r#"{"status":null,"output":null,"usage":null,"error":null,"#,
                r#""incomplete_details":null,"model":null}"#
            )),
        )
        .await
        .expect("null values leave the state untouched");
    assert_eq!(support::stored(cache, ID), before);
}

pub(super) fn unknown_field_is_rejected() {
    let fields = IndexMap::from([("bogus".to_owned(), support::json("1"))]);
    assert_eq!(
        PollPatch::try_from(fields),
        Err(UnknownPatchField {
            field: "bogus".to_owned()
        })
    );
}

pub(super) fn ttl_is_refreshed(cache: &FakeCache) {
    let effects = cache.effects();
    let mut writes = 0_usize;
    for effect in &effects {
        if effect.operation != CacheOperation::Write {
            continue;
        }
        writes = writes.saturating_add(1);
        assert_eq!(effect.key, get_cache_key(ID));
        assert_eq!(effect.ttl, Some(TTL));
    }
    assert_eq!(writes, 6);
    assert_eq!(cache.count(CacheOperation::Read), 6);
    let final_write = effects.last().expect("the last effect is the final write");
    let stored = cache.entry(&final_write.key).expect("the write landed");
    assert_eq!(final_write.value.as_deref(), Some(stored.as_str()));
}

pub(super) fn assert_logs(watch: &Watch) {
    assert_eq!(
        watch.messages_at("DEBUG"),
        vec![
            "Updated polling state for litellm_poll_patch: status=queued, output_items=1",
            "Updated polling state for litellm_poll_patch: status=queued, output_items=0",
            "Updated polling state for litellm_poll_patch: status=queued, output_items=0",
            "Updated polling state for litellm_poll_patch: status=failed, output_items=0",
            "Updated polling state for litellm_poll_patch: status=failed, output_items=0",
            "Updated polling state for litellm_poll_patch: status=failed, output_items=0",
        ]
    );
}

pub(super) fn seed(cache: &FakeCache) {
    cache.seed(&get_cache_key(ID), SEED);
}
