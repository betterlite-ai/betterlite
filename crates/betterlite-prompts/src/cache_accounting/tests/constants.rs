use crate::cache_accounting::constants::{
    BREAKPOINT_BLOCK_TYPES, CACHE_BREAKPOINT_KEYS, CARRY_UNMATCHED_MESSAGE_POINTS,
    EXTERNAL_BREAKPOINTS_STAMP, GATEWAY_CACHE_METADATA_KEY, GPT_VERSION_PATTERN,
    INJECTED_FOR_EVERY_DEPLOYMENT, MAX_CACHE_CONTROL_BLOCKS, MIN_BREAKPOINT_GPT_VERSION,
    OPENAI_API_BASE_ENV_VARS, OPENAI_API_HOST, OPENAI_DIALECT_STAMP,
};
use crate::cache_accounting::points::{CacheControlInjectionPoint, MessageInjectionPoint};
use crate::cache_accounting::tests::support::json;

#[test]
fn the_published_keys_keep_litellms_spelling() {
    assert_eq!(
        CACHE_BREAKPOINT_KEYS,
        ["cache_control", "prompt_cache_breakpoint"]
    );
    assert_eq!(MAX_CACHE_CONTROL_BLOCKS, 4);
    assert_eq!(MIN_BREAKPOINT_GPT_VERSION, (5, 6));
    assert_eq!(GPT_VERSION_PATTERN, r"^gpt-(\d+)(?:\.(\d+))?");
    assert_eq!(OPENAI_API_HOST, "api.openai.com");
    assert_eq!(
        OPENAI_API_BASE_ENV_VARS,
        ["OPENAI_BASE_URL", "OPENAI_API_BASE"]
    );
    assert_eq!(
        CARRY_UNMATCHED_MESSAGE_POINTS,
        "_litellm_carry_unmatched_cache_control_points"
    );
    assert_eq!(EXTERNAL_BREAKPOINTS_STAMP, "_litellm_external_breakpoints");
    assert_eq!(OPENAI_DIALECT_STAMP, "_litellm_openai_dialect");
    assert_eq!(GATEWAY_CACHE_METADATA_KEY, "litellm_gateway_injected_cache");
    assert_eq!(INJECTED_FOR_EVERY_DEPLOYMENT, "");
    assert_eq!(BREAKPOINT_BLOCK_TYPES.len(), 8);
}

#[test]
fn the_stamp_fields_serialize_under_the_published_keys() {
    let stamped = CacheControlInjectionPoint::Message(MessageInjectionPoint {
        role: None,
        index: None,
        control: None,
        openai_dialect: None,
        external_breakpoints: None,
    })
    .restamped(Some(true), Some(1));
    let rendered = json(&serde_json::to_string(&stamped).unwrap());
    let fields = rendered.as_object().unwrap();
    assert_eq!(fields.get(OPENAI_DIALECT_STAMP), Some(&json("true")));
    assert_eq!(fields.get(EXTERNAL_BREAKPOINTS_STAMP), Some(&json("1")));
}
