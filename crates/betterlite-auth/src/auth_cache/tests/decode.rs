use std::fs;
use std::path::PathBuf;

use betterlite_models::decisions::Json;
use betterlite_redis::cache_primitives::decode::{CachedValue, decode_cached_value};
use rstest::rstest;

use super::support::{capture, local_cache};
use crate::auth_cache::WriteOptions;
use crate::auth_cache::decode::{
    MAX_LITERAL_DEPTH, MAX_PAYLOAD_BYTES, MAX_PAYLOAD_DEPTH, decode_cached_auth, depth,
};
use crate::auth_cache::payload::ReadOperation;

fn auth_payload(bytes: usize) -> Vec<u8> {
    let prefix = "{\"team_id\":\"";
    let suffix = "\"}";
    let fill = bytes
        .saturating_sub(prefix.len())
        .saturating_sub(suffix.len());
    let mut text = String::from(prefix);
    text.push_str(&"a".repeat(fill));
    text.push_str(suffix);
    text.into_bytes()
}

fn wrapped(core: &str, depth: usize) -> Vec<u8> {
    let mut text = core.to_owned();
    for _level in 0..depth {
        text = format!("[{text}]");
    }
    text.into_bytes()
}

#[test]
fn auth_cache_payload_over_byte_limit() {
    let allowed = decode_cached_auth(&auth_payload(MAX_PAYLOAD_BYTES), ReadOperation::Get);
    assert!(matches!(allowed, Ok(Some(_))), "{allowed:?}");
    let (over, log) = capture(|| {
        decode_cached_auth(
            &auth_payload(MAX_PAYLOAD_BYTES.saturating_add(1)),
            ReadOperation::Get,
        )
    });
    assert!(over.unwrap().is_none());
    assert!(log.contains("exceeds the cache payload limits"), "{log}");
}

#[test]
fn auth_cache_payload_over_depth_limit() {
    let at_limit = decode_cached_value(Some(&wrapped("1", MAX_PAYLOAD_DEPTH)))
        .unwrap()
        .unwrap();
    assert_eq!(depth(&at_limit), MAX_PAYLOAD_DEPTH);
    let (allowed, allowed_log) =
        capture(|| decode_cached_auth(&wrapped("1", MAX_PAYLOAD_DEPTH), ReadOperation::Get));
    assert!(allowed.unwrap().is_none());
    assert!(!allowed_log.contains("depth limit"), "{allowed_log}");
    let (over, log) = capture(|| {
        decode_cached_auth(
            &wrapped("1", MAX_PAYLOAD_DEPTH.saturating_add(1)),
            ReadOperation::Get,
        )
    });
    assert!(over.unwrap().is_none());
    assert!(
        log.contains("exceeds the cache payload depth limit"),
        "{log}"
    );
}

#[test]
fn literal_fallback_nesting_is_bounded() {
    let (over, log) = capture(|| {
        decode_cached_auth(
            &wrapped("None", MAX_LITERAL_DEPTH.saturating_add(1)),
            ReadOperation::Get,
        )
    });
    assert!(over.unwrap().is_none());
    assert!(log.contains("literal nesting limit"), "{log}");
    let (deep, deep_log) = capture(|| decode_cached_auth(&wrapped("None", 24), ReadOperation::Get));
    assert!(deep.unwrap().is_none());
    assert!(deep_log.contains("literal nesting limit"), "{deep_log}");
    let (at_limit, at_log) =
        capture(|| decode_cached_auth(&wrapped("None", MAX_LITERAL_DEPTH), ReadOperation::Get));
    assert!(at_limit.unwrap().is_none());
    assert!(!at_log.contains("literal nesting limit"), "{at_log}");
    let siblings = b"[None],[None],[None],[None],[None],[None],[None],[None],[None],[None]";
    let (flat, flat_log) = capture(|| decode_cached_auth(siblings, ReadOperation::Get));
    assert!(flat.unwrap().is_none());
    assert!(!flat_log.contains("literal nesting limit"), "{flat_log}");
}

#[test]
fn json_nesting_is_not_held_to_the_literal_bound() {
    let (decoded, log) = capture(|| {
        decode_cached_auth(
            &wrapped("1", MAX_LITERAL_DEPTH.saturating_add(4)),
            ReadOperation::Get,
        )
    });
    assert!(decoded.unwrap().is_none());
    assert!(!log.contains("literal nesting limit"), "{log}");
}

#[test]
fn literal_brackets_inside_strings_do_not_count() {
    let payload = br#"{"team_id": 'it\'s ((((((((((', "api_key": None}"#;
    let decoded = decode_cached_auth(payload, ReadOperation::Get)
        .unwrap()
        .expect("the literal payload decodes");
    assert_eq!(
        decoded.view.token.team_id.as_deref(),
        Some("it's ((((((((((")
    );
}

#[test]
fn literal_nesting_is_counted_after_a_closed_string() {
    let payload = b"{'a': 'x', 'b': [[[[[[[[None]]]]]]]]}";
    let (decoded, log) = capture(|| decode_cached_auth(payload, ReadOperation::Get));
    assert!(decoded.unwrap().is_none());
    assert!(log.contains("literal nesting limit"), "{log}");
}

#[test]
fn malformed_bytes_become_typed_miss() {
    let (decoded, log) = capture(|| decode_cached_auth(b"{oops", ReadOperation::Get));
    assert!(decoded.unwrap().is_none());
    assert!(log.contains("is not decodable"), "{log}");
    assert!(log.contains("literal_eval"), "{log}");
    let (invalid_utf8, utf8_log) =
        capture(|| decode_cached_auth(&[0xff, 0xfe, 0x00], ReadOperation::AsyncGet));
    assert!(invalid_utf8.unwrap().is_none());
    assert!(utf8_log.contains("utf8"), "{utf8_log}");
    let absent = decode_cached_auth(b"", ReadOperation::Get);
    assert!(absent.unwrap().is_none());
}

#[test]
fn duplicate_fields_use_shared_decoder() {
    let payload = br#"{"team_id":"first","team_id":"second","api_key":null}"#;
    let shared = decode_cached_value(Some(payload)).unwrap().unwrap();
    let CachedValue::Object(fields) = &shared else {
        panic!("the payload is an object");
    };
    assert_eq!(fields.len(), 2);
    let decoded = decode_cached_auth(payload, ReadOperation::Get)
        .unwrap()
        .expect("the duplicated payload decodes");
    assert_eq!(decoded.view.token.team_id.as_deref(), Some("second"));
}

#[test]
fn caller_markers_not_promoted() {
    let payload = br#"{"team_id":"team-1","requires_fresh_policy":true,"via_virtual_key":true,"authenticated_by_custom_auth":true,"mcp_admitted_user_subject":true,"api_key":null}"#;
    let decoded = decode_cached_auth(payload, ReadOperation::Get)
        .unwrap()
        .expect("the marked payload decodes");
    assert!(!decoded.extra.requires_fresh_policy);
    assert!(!decoded.extra.provenance.via_virtual_key);
    assert!(!decoded.extra.provenance.authenticated_by_custom_auth);
    assert!(!decoded.extra.mcp_access.mcp_admitted_user_subject);
    assert_eq!(decoded.view.token.team_id.as_deref(), Some("team-1"));
}

#[test]
fn cache_diagnostics_redact_payload_and_key() {
    let payload = br#"{"max_budget":"sk-live-secret"}"#;
    let (decoded, log) = capture(|| decode_cached_auth(payload, ReadOperation::Get));
    assert!(decoded.unwrap().is_none());
    assert!(
        log.contains("CacheCodec.deserialize: validation failed"),
        "{log}"
    );
    assert!(!log.contains("sk-live-secret"), "{log}");
    assert!(log.contains("view") || log.contains("json"), "{log}");
    let cache = local_cache(Some(60.0_f64));
    let key = "team_membership:user-1:team-1";
    let stored: Json = serde_json::from_str(r#"{"max_budget":"sk-live-secret"}"#).unwrap();
    cache
        .set(Some(key), &stored, WriteOptions::default())
        .unwrap();
    let (missing, keyed_log) = capture(|| cache.get_auth(key, false));
    assert!(missing.unwrap().is_none());
    assert!(!keyed_log.contains("sk-live-secret"), "{keyed_log}");
    assert!(!keyed_log.contains(key), "{keyed_log}");
    assert!(
        keyed_log.contains("UserApiKeyCache.get_cache"),
        "{keyed_log}"
    );
}

#[rstest]
fn replays_the_auth_cache_corpus(#[files("fuzz/corpus/auth_cache/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    drop(decode_cached_auth(&bytes, ReadOperation::Get));
    drop(decode_cached_auth(&bytes, ReadOperation::AsyncGet));
}
