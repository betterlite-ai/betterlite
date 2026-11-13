use crate::arguments::json::{Json, Object};
use crate::bedrock_tools::capabilities::{BedrockToolCapabilities, CachePointSupport};
use crate::bedrock_tools::error::BedrockToolError;
use crate::bedrock_tools::schema::{ExpansionBudget, parameter_schema};
use crate::definitions::legacy::DEFS_MAX_INLINED_BYTES;

use super::prelude::json;

enum Expected {
    Schema(&'static str),
    WrongType(&'static str),
    ExpansionFailed,
}

enum Scenario {
    Case {
        parameters: &'static str,
        supports_strict_tools: bool,
        expected: Expected,
    },
    ExpansionBomb,
}

#[rstest::rstest]
#[case::projects_the_supported_keys(Scenario::Case {
    parameters: r#"{"type":"object","title":"t","description":"d","properties":{"a":{"type":"string"}},"required":["a"],"additionalProperties":false}"#,
    supports_strict_tools: true,
    expected: Expected::Schema(r#"{"type":"object","properties":{"a":{"type":"string"}},"required":["a"],"additionalProperties":false}"#),
})]
#[case::defaults_a_missing_type(Scenario::Case {
    parameters: r#"{"properties":{}}"#,
    supports_strict_tools: false,
    expected: Expected::Schema(r#"{"type":"object","properties":{},"required":[]}"#),
})]
#[case::defaults_a_null_type(Scenario::Case {
    parameters: r#"{"type":null}"#,
    supports_strict_tools: false,
    expected: Expected::Schema(r#"{"type":"object","properties":{},"required":[]}"#),
})]
#[case::keeps_a_valid_non_object_root_type(Scenario::Case {
    parameters: r#"{"type":"array"}"#,
    supports_strict_tools: false,
    expected: Expected::Schema(r#"{"type":"array","properties":{},"required":[]}"#),
})]
#[case::replaces_an_unknown_root_type(Scenario::Case {
    parameters: r#"{"type":"any"}"#,
    supports_strict_tools: false,
    expected: Expected::Schema(r#"{"type":"object","properties":{},"required":[]}"#),
})]
#[case::replaces_a_custom_root_type(Scenario::Case {
    parameters: r#"{"type":"custom"}"#,
    supports_strict_tools: false,
    expected: Expected::Schema(r#"{"type":"object","properties":{},"required":[]}"#),
})]
#[case::rejects_a_non_string_type(Scenario::Case {
    parameters: r#"{"type":5}"#,
    supports_strict_tools: false,
    expected: Expected::WrongType("type"),
})]
#[case::defaults_missing_properties(Scenario::Case {
    parameters: r#"{"type":"object"}"#,
    supports_strict_tools: false,
    expected: Expected::Schema(r#"{"type":"object","properties":{},"required":[]}"#),
})]
#[case::defaults_null_properties(Scenario::Case {
    parameters: r#"{"type":"object","properties":null}"#,
    supports_strict_tools: false,
    expected: Expected::Schema(r#"{"type":"object","properties":{},"required":[]}"#),
})]
#[case::rejects_non_object_properties(Scenario::Case {
    parameters: r#"{"type":"object","properties":[]}"#,
    supports_strict_tools: false,
    expected: Expected::WrongType("properties"),
})]
#[case::defaults_missing_required(Scenario::Case {
    parameters: r#"{"type":"object"}"#,
    supports_strict_tools: false,
    expected: Expected::Schema(r#"{"type":"object","properties":{},"required":[]}"#),
})]
#[case::defaults_null_required(Scenario::Case {
    parameters: r#"{"type":"object","required":null}"#,
    supports_strict_tools: false,
    expected: Expected::Schema(r#"{"type":"object","properties":{},"required":[]}"#),
})]
#[case::keeps_required(Scenario::Case {
    parameters: r#"{"type":"object","required":["a","b"]}"#,
    supports_strict_tools: false,
    expected: Expected::Schema(r#"{"type":"object","properties":{},"required":["a","b"]}"#),
})]
#[case::rejects_a_required_with_non_strings(Scenario::Case {
    parameters: r#"{"type":"object","required":["a",1]}"#,
    supports_strict_tools: false,
    expected: Expected::WrongType("required"),
})]
#[case::rejects_a_non_list_required(Scenario::Case {
    parameters: r#"{"type":"object","required":"a"}"#,
    supports_strict_tools: false,
    expected: Expected::WrongType("required"),
})]
#[case::keeps_a_boolean_additional_property_for_strict_tools(Scenario::Case {
    parameters: r#"{"type":"object","additionalProperties":false}"#,
    supports_strict_tools: true,
    expected: Expected::Schema(r#"{"type":"object","properties":{},"required":[],"additionalProperties":false}"#),
})]
#[case::keeps_a_schema_additional_property_for_strict_tools(Scenario::Case {
    parameters: r#"{"type":"object","additionalProperties":{"type":"string"}}"#,
    supports_strict_tools: true,
    expected: Expected::Schema(r#"{"type":"object","properties":{},"required":[],"additionalProperties":{"type":"string"}}"#),
})]
#[case::drops_additional_properties_without_strict_tools(Scenario::Case {
    parameters: r#"{"type":"object","additionalProperties":false}"#,
    supports_strict_tools: false,
    expected: Expected::Schema(r#"{"type":"object","properties":{},"required":[]}"#),
})]
#[case::drops_a_null_additional_property(Scenario::Case {
    parameters: r#"{"type":"object","additionalProperties":null}"#,
    supports_strict_tools: true,
    expected: Expected::Schema(r#"{"type":"object","properties":{},"required":[]}"#),
})]
#[case::expands_a_local_ref_and_removes_defs(Scenario::Case {
    parameters: r##"{"type":"object","properties":{"x":{"$ref":"#/$defs/Count"}},"$defs":{"Count":{"type":"integer"}}}"##,
    supports_strict_tools: false,
    expected: Expected::Schema(r#"{"type":"object","properties":{"x":{"type":"integer"}},"required":[]}"#),
})]
#[case::expands_a_legacy_definitions_ref(Scenario::Case {
    parameters: r##"{"type":"object","properties":{"x":{"$ref":"#/definitions/Count"}},"definitions":{"Count":{"type":"integer"}}}"##,
    supports_strict_tools: false,
    expected: Expected::Schema(r#"{"type":"object","properties":{"x":{"type":"integer"}},"required":[]}"#),
})]
#[case::treats_null_defs_as_absent(Scenario::Case {
    parameters: r#"{"type":"object","$defs":null}"#,
    supports_strict_tools: false,
    expected: Expected::Schema(r#"{"type":"object","properties":{},"required":[]}"#),
})]
#[case::rejects_non_object_defs(Scenario::Case {
    parameters: r#"{"type":"object","$defs":[]}"#,
    supports_strict_tools: false,
    expected: Expected::WrongType("$defs"),
})]
#[case::replaces_a_nested_custom_property_type(Scenario::Case {
    parameters: r#"{"type":"object","properties":{"q":{"type":"custom"}}}"#,
    supports_strict_tools: false,
    expected: Expected::Schema(r#"{"type":"object","properties":{"q":{"type":"object"}},"required":[]}"#),
})]
#[case::replaces_custom_types_below_items_and_combinators(Scenario::Case {
    parameters: r#"{"type":"object","properties":{"q":{"type":"array","items":{"type":"custom"},"anyOf":[{"type":"custom"},{"type":"string"}]}}}"#,
    supports_strict_tools: false,
    expected: Expected::Schema(r#"{"type":"object","properties":{"q":{"type":"array","items":{"type":"object"},"anyOf":[{"type":"object"},{"type":"string"}]}},"required":[]}"#),
})]
#[case::replaces_a_custom_type_in_additional_properties(Scenario::Case {
    parameters: r#"{"type":"object","additionalProperties":{"type":"custom"}}"#,
    supports_strict_tools: true,
    expected: Expected::Schema(r#"{"type":"object","properties":{},"required":[],"additionalProperties":{"type":"object"}}"#),
})]
#[case::rejects_the_expansion_budget(Scenario::ExpansionBomb)]
fn bedrock_tools_behavior_tests_behavior(#[case] scenario: Scenario) {
    let (parameters, supports_strict_tools, expected) = match scenario {
        Scenario::Case {
            parameters,
            supports_strict_tools,
            expected,
        } => (json(parameters), supports_strict_tools, expected),
        Scenario::ExpansionBomb => (expansion_bomb(), true, Expected::ExpansionFailed),
    };
    let capabilities = BedrockToolCapabilities {
        cache_points: CachePointSupport::Unsupported,
        supports_strict_tools,
    };
    let mut budget = ExpansionBudget::new(DEFS_MAX_INLINED_BYTES);
    let result = parameter_schema(&parameters, capabilities, &mut budget);
    match expected {
        Expected::Schema(raw) => assert_eq!(result.unwrap(), json(raw)),
        Expected::WrongType(field) => assert!(matches!(
            result,
            Err(BedrockToolError::WrongType { field: actual }) if actual == field
        )),
        Expected::ExpansionFailed => {
            assert!(matches!(result, Err(BedrockToolError::Schema { .. })));
        }
    }
}

fn expansion_bomb() -> Json {
    let big = "x".repeat(6_000_000);
    let mut properties = Object::new();
    for key in ["a", "b"] {
        properties.insert(key.to_owned(), json(r##"{"$ref":"#/$defs/Big"}"##));
    }
    Json::Object(Object::from_iter([
        ("type".to_owned(), Json::Text("object".to_owned())),
        ("properties".to_owned(), Json::Object(properties)),
        (
            "$defs".to_owned(),
            Json::Object(Object::from_iter([(
                "Big".to_owned(),
                Json::Object(Object::from_iter([
                    ("type".to_owned(), Json::Text("string".to_owned())),
                    ("description".to_owned(), Json::Text(big)),
                ])),
            )])),
        ),
    ]))
}
