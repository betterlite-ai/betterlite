use crate::api::messages_chat_prepare::inspect::{
    extract_proxy_metadata, has_compaction_block,
    inspect_compaction_and_proxy_context as inspect_context,
};

use super::support::{NeverCalled, decided, extra, object, parse};

const COMPACTION: &str = r#"{"type":"compaction","content":"S"}"#;

#[test]
fn inspect_compaction_and_proxy_context() {
    assert_block_shapes();
    assert_block_rejections();
    assert_proxy_context();
    assert_proxy_absences();
    assert_inspection_router();
}

// a compaction dict block inside a list content is the only shape that counts, on any role
fn assert_block_shapes() {
    let assistant = decided(&format!(
        r#"[{{"role":"assistant","content":[{{"type":"text","text":"done"}},{COMPACTION}]}}]"#
    ));
    assert!(has_compaction_block(&assistant));
    let user = decided(&format!(r#"[{{"role":"user","content":[{COMPACTION}]}}]"#));
    assert!(has_compaction_block(&user));
    let system = decided(&format!(
        r#"[{{"role":"system","content":[{COMPACTION}]}}]"#
    ));
    assert!(has_compaction_block(&system));
    let later = decided(&format!(
        r#"[{{"role":"user","content":"hello"}},{{"role":"assistant","content":[{COMPACTION}]}}]"#
    ));
    assert!(has_compaction_block(&later));
}

// scalar text that names compaction, dict content, other block types and non-text types are all false
fn assert_block_rejections() {
    assert!(!has_compaction_block(&decided(
        r#"[{"role":"assistant","content":"a compaction happened"}]"#
    )));
    assert!(!has_compaction_block(&decided(&format!(
        r#"[{{"role":"assistant","content":{COMPACTION}}}]"#
    ))));
    assert!(!has_compaction_block(&decided(
        r#"[{"role":"assistant","content":[{"type":"text","text":"compaction"}]}]"#
    )));
    assert!(!has_compaction_block(&decided(
        r#"[{"role":"assistant","content":[{"type":7,"content":"S"}]}]"#
    )));
    assert!(!has_compaction_block(&decided(
        r#"[{"role":"assistant","content":["compaction"]}]"#
    )));
    assert!(!has_compaction_block(&decided(r#"[{"role":"assistant"}]"#)));
    assert!(!has_compaction_block(&[]));
}

// the proxy map yields its exact fields and the raw auth entry it carries
fn assert_proxy_context() {
    let kwargs = extra(
        r#"{"litellm_metadata":{"user_id":"u-1","user_api_key_auth":{"api_key":"sk-1"},"litellm_call_id":"c-1"},"temperature":0.5}"#,
    );
    let context = extract_proxy_metadata(&kwargs).expect("the mapping is a proxy context");
    assert_eq!(
        context.litellm_metadata,
        object(
            r#"{"user_id":"u-1","user_api_key_auth":{"api_key":"sk-1"},"litellm_call_id":"c-1"}"#
        )
    );
    assert_eq!(
        context.user_api_key_auth,
        Some(parse(r#"{"api_key":"sk-1"}"#))
    );

    // an empty proxy map, or one without the auth entry, still yields (metadata, null)
    let empty = extract_proxy_metadata(&extra(r#"{"litellm_metadata":{}}"#)).expect("a context");
    assert!(empty.litellm_metadata.is_empty());
    assert!(empty.user_api_key_auth.is_none());
    let anthropic = extract_proxy_metadata(&extra(r#"{"litellm_metadata":{"user_id":"u-1"}}"#))
        .expect("a context");
    assert_eq!(anthropic.litellm_metadata, object(r#"{"user_id":"u-1"}"#));
    assert!(anthropic.user_api_key_auth.is_none());
}

// a missing, null or non-dict litellm_metadata is no proxy context, even with other kwargs
fn assert_proxy_absences() {
    assert!(extract_proxy_metadata(&extra(r#"{"temperature":0.5}"#)).is_none());
    assert!(extract_proxy_metadata(&extra(r#"{"litellm_metadata":null}"#)).is_none());
    assert!(extract_proxy_metadata(&extra(r#"{"litellm_metadata":"text"}"#)).is_none());
    assert!(extract_proxy_metadata(&extra(r#"{"litellm_metadata":[1,2]}"#)).is_none());
}

// the router handle comes back as the caller passed it, and nothing answers when it is absent
fn assert_inspection_router() {
    let kwargs = extra(r#"{"litellm_metadata":{"user_id":"u-1"}}"#);
    let assistant = decided(&format!(
        r#"[{{"role":"assistant","content":[{{"type":"text","text":"done"}},{COMPACTION}]}}]"#
    ));
    let router = NeverCalled;
    let inspection = inspect_context(&assistant, &kwargs, Some(&router));
    assert!(inspection.has_compaction_block);
    assert!(inspection.proxy.is_some());
    assert!(inspection.router.is_some());
    let absent = inspect_context(&assistant, &extra("{}"), None);
    assert!(absent.router.is_none());
    assert!(absent.proxy.is_none());
    let unavailable = inspect_context(&decided("[]"), &extra("{}"), None);
    assert!(unavailable.router.is_none());
    assert!(!unavailable.has_compaction_block);
}
