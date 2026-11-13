use crate::bedrock_tools::cache_point::add_cache_point_tool_block;
use crate::bedrock_tools::capabilities::CachePointSupport;
use crate::bedrock_tools::error::BedrockToolError;

use super::prelude::{json, object};

const UNSUPPORTED: CachePointSupport = CachePointSupport::Unsupported;
const SHORT: CachePointSupport = CachePointSupport::ShortTtl;
const EXTENDED: CachePointSupport = CachePointSupport::ExtendedTtl;

enum Expected {
    Block(&'static str),
    Missing,
    WrongType,
}

struct Scenario {
    tool: &'static str,
    support: CachePointSupport,
    expected: Expected,
}

#[rstest::rstest]
#[case::an_absent_control_is_no_block(Scenario { tool: "{}", support: SHORT, expected: Expected::Missing })]
#[case::a_null_control_is_no_block(Scenario { tool: r#"{"cache_control":null}"#, support: SHORT, expected: Expected::Missing })]
#[case::a_non_object_control_is_a_typed_failure(Scenario { tool: r#"{"cache_control":"ephemeral"}"#, support: SHORT, expected: Expected::WrongType })]
#[case::an_unsupported_model_gets_no_block(Scenario { tool: r#"{"cache_control":{"type":"ephemeral","ttl":"1h"}}"#, support: UNSUPPORTED, expected: Expected::Missing })]
#[case::an_absent_type_is_ephemeral(Scenario { tool: r#"{"cache_control":{}}"#, support: SHORT, expected: Expected::Block(r#"{"cachePoint":{"type":"default"}}"#) })]
#[case::a_null_type_is_ephemeral(Scenario { tool: r#"{"cache_control":{"type":null}}"#, support: SHORT, expected: Expected::Block(r#"{"cachePoint":{"type":"default"}}"#) })]
#[case::an_unknown_type_is_no_block(Scenario { tool: r#"{"cache_control":{"type":"persistent"}}"#, support: SHORT, expected: Expected::Missing })]
#[case::a_short_ttl_model_ignores_one_hour(Scenario { tool: r#"{"cache_control":{"type":"ephemeral","ttl":"1h"}}"#, support: SHORT, expected: Expected::Block(r#"{"cachePoint":{"type":"default"}}"#) })]
#[case::a_short_ttl_model_ignores_five_minutes(Scenario { tool: r#"{"cache_control":{"type":"ephemeral","ttl":"5m"}}"#, support: SHORT, expected: Expected::Block(r#"{"cachePoint":{"type":"default"}}"#) })]
#[case::an_extended_ttl_model_keeps_one_hour(Scenario { tool: r#"{"cache_control":{"type":"ephemeral","ttl":"1h"}}"#, support: EXTENDED, expected: Expected::Block(r#"{"cachePoint":{"type":"default","ttl":"1h"}}"#) })]
#[case::an_extended_ttl_model_keeps_five_minutes(Scenario { tool: r#"{"cache_control":{"type":"ephemeral","ttl":"5m"}}"#, support: EXTENDED, expected: Expected::Block(r#"{"cachePoint":{"type":"default","ttl":"5m"}}"#) })]
#[case::an_extended_ttl_model_drops_an_unknown_ttl(Scenario { tool: r#"{"cache_control":{"type":"ephemeral","ttl":"10m"}}"#, support: EXTENDED, expected: Expected::Block(r#"{"cachePoint":{"type":"default"}}"#) })]
#[case::a_null_ttl_is_no_ttl(Scenario { tool: r#"{"cache_control":{"type":"ephemeral","ttl":null}}"#, support: EXTENDED, expected: Expected::Block(r#"{"cachePoint":{"type":"default"}}"#) })]
#[case::a_non_string_ttl_is_no_ttl(Scenario { tool: r#"{"cache_control":{"type":"ephemeral","ttl":5}}"#, support: EXTENDED, expected: Expected::Block(r#"{"cachePoint":{"type":"default"}}"#) })]
#[case::a_control_without_a_type_still_gets_a_ttl(Scenario { tool: r#"{"cache_control":{"ttl":"1h"}}"#, support: EXTENDED, expected: Expected::Block(r#"{"cachePoint":{"type":"default","ttl":"1h"}}"#) })]
fn bedrock_tools_behavior_tests_behavior(#[case] scenario: Scenario) {
    let result = add_cache_point_tool_block(&object(scenario.tool), scenario.support);
    match scenario.expected {
        Expected::Block(raw) => assert_eq!(result.unwrap().unwrap(), json(raw)),
        Expected::Missing => assert!(matches!(result, Ok(None))),
        Expected::WrongType => assert!(matches!(
            result,
            Err(BedrockToolError::WrongType {
                field: "cache_control"
            })
        )),
    }
}
