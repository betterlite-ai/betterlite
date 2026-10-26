use super::{
    ConversationCache, LassoError, LassoGuardrail, MemoryCacheEntry, extract_tool_arguments,
};
use serde_json::json;
use std::cell::RefCell;
use std::collections::BTreeMap;

#[test]
fn lasso_guardrail_constructor_requires_api_key_and_defaults_base() {
    assert!(matches!(
        LassoGuardrail::new(None, None, None, None, false),
        Err(LassoError::MissingSecrets)
    ));
    let guardrail = LassoGuardrail::new(Some("key".to_owned()), None, None, None, false).unwrap();
    assert_eq!(
        guardrail.api_base,
        "https://server.lasso.security/gateway/v3"
    );
}

#[test]
fn lasso_guardrail_session_cache_uses_call_id_and_ttl() {
    let cache = RefCell::new(BTreeMap::<String, MemoryCacheEntry>::new());
    let guardrail = LassoGuardrail::new(Some("key".to_owned()), None, None, None, false).unwrap();
    let generated = guardrail.conversation_id(Some("call-1"), &cache);
    let cached = guardrail.conversation_id(Some("call-1"), &cache);
    assert_eq!(generated, cached);
    let entry = cache
        .borrow()
        .get("lasso_conversation_id:call-1")
        .cloned()
        .unwrap();
    assert_eq!(entry.ttl_seconds, 3600);
}

#[test]
fn lasso_guardrail_preserves_malformed_tool_arguments() {
    let call = json!({"function": {"arguments": "smuggled PII"}});
    assert_eq!(
        extract_tool_arguments(&call),
        Some(json!({"arguments": "smuggled PII"}))
    );
}

#[test]
fn lasso_guardrail_only_blocks_block_actions() {
    let guardrail = LassoGuardrail::new(Some("key".to_owned()), None, None, None, false).unwrap();
    let warning =
        json!({"violations_detected": true, "findings": {"pii": [{"action": "AUTO_MASKING"}]} });
    assert!(guardrail.process_response(warning).is_ok());
    let blocked =
        json!({"violations_detected": true, "findings": {"jailbreak": [{"action": "BLOCK"}]} });
    assert!(matches!(
        guardrail.process_response(blocked),
        Err(LassoError::Blocked { .. })
    ));
}

#[test]
fn lasso_guardrail_cache_trait_is_usable() {
    let cache = RefCell::new(BTreeMap::<String, MemoryCacheEntry>::new());
    cache.set_cache("key", "value", 3600).unwrap();
    assert_eq!(cache.get_cache("key").unwrap(), Some("value".to_owned()));
}
