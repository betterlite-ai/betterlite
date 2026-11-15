#![cfg(test)]

use rstest::rstest;
use serde_json::{Map, Value, json};

use crate::guardrails::provider_forms::core_tables::core_tables;
use crate::guardrails::provider_forms::error::FormError;
use crate::guardrails::provider_forms::project::{project_model, with_friendly_name};
use crate::guardrails::provider_forms::spec::{FormExtra, FormField, FormModel, FormShape};

fn model(fields: Vec<FormField>) -> FormModel {
    FormModel {
        ui_friendly_name: None,
        fields,
    }
}

fn field(name: &'static str, shape: FormShape) -> FormField {
    FormField {
        name,
        description: None,
        required: false,
        shape,
        default: None,
        extra: FormExtra::NONE,
    }
}

fn text_field(name: &'static str, description: Option<&'static str>) -> FormField {
    FormField {
        name,
        description,
        required: false,
        shape: FormShape::Text,
        default: None,
        extra: FormExtra::NONE,
    }
}

fn project(fields: Vec<FormField>) -> Map<String, Value> {
    project_model(&model(fields), 100).unwrap()
}

fn chain(length: usize) -> FormModel {
    let mut current = model(vec![text_field("leaf", None)]);
    for _ in 0..length {
        let next = FormField {
            name: "next",
            description: None,
            required: false,
            shape: FormShape::Nested(Box::new(current)),
            default: None,
            extra: FormExtra::NONE,
        };
        current = model(vec![next]);
    }
    current
}

#[test]
fn projection_description_falls_back_to_the_field_name() {
    let fields = project(vec![
        text_field("declared", Some("The declared text")),
        text_field("bare", None),
    ]);
    assert_eq!(
        fields["declared"]["description"],
        json!("The declared text")
    );
    assert_eq!(fields["bare"]["description"], json!("bare"));
    assert_eq!(fields["bare"]["required"], json!(false));
}

#[rstest]
#[case::text(FormShape::Text, "string")]
#[case::number(FormShape::Number, "number")]
#[case::boolean(FormShape::Boolean, "boolean")]
#[case::mapping(
    FormShape::Mapping { keys: Vec::new(), value: Box::new(FormShape::Other) },
    "dict"
)]
#[case::list(FormShape::List { options: Vec::new() }, "array")]
#[case::choice(FormShape::Choice(Vec::new()), "select")]
#[case::raw_object(FormShape::RawObject, "object")]
#[case::raw_list(FormShape::RawList, "array")]
#[case::other(FormShape::Other, "string")]
fn projection_type_strings_follow_the_shape(#[case] shape: FormShape, #[case] expected: &str) {
    let fields = project(vec![field("value", shape)]);
    assert_eq!(fields["value"]["type"], json!(expected));
}

#[test]
fn projection_required_fields_keep_their_flag() {
    let required = FormField {
        name: "mandatory",
        description: None,
        required: true,
        shape: FormShape::Text,
        default: None,
        extra: FormExtra::NONE,
    };
    let fields = project(vec![required]);
    assert_eq!(fields["mandatory"]["required"], json!(true));
}

#[test]
fn projection_nested_field_gets_type_and_fields() {
    let inner = model(vec![text_field("label", Some("Entity label"))]);
    let outer = FormField {
        name: "entity",
        description: Some("Primary entity"),
        required: false,
        shape: FormShape::Nested(Box::new(inner)),
        default: None,
        extra: FormExtra::NONE,
    };
    let fields = project(vec![outer]);
    assert_eq!(fields["entity"]["description"], json!("Primary entity"));
    assert_eq!(fields["entity"]["type"], json!("nested"));
    assert_eq!(fields["entity"]["fields"]["label"]["type"], json!("string"));
    assert!(fields["entity"]["fields"]["label"].get("options").is_none());
}

#[test]
fn projection_list_inside_a_nested_model_stays_an_array() {
    let inner = model(vec![FormField {
        name: "labels",
        description: None,
        required: false,
        shape: FormShape::List {
            options: Vec::new(),
        },
        default: None,
        extra: FormExtra::NONE,
    }]);
    let outer = FormField {
        name: "entity",
        description: None,
        required: false,
        shape: FormShape::Nested(Box::new(inner)),
        default: None,
        extra: FormExtra::NONE,
    };
    let fields = project(vec![outer]);
    assert_eq!(fields["entity"]["fields"]["labels"]["type"], json!("array"));
}

#[test]
fn projection_ui_type_replaces_the_type() {
    let flag = FormField {
        name: "output_parse_pii",
        description: None,
        required: false,
        shape: FormShape::Boolean,
        default: None,
        extra: FormExtra {
            ui_type: Some("bool".to_owned()),
            type_override: None,
            options: None,
            min: None,
            max: None,
            step: None,
            default_value: None,
        },
    };
    let fields = project(vec![flag]);
    assert_eq!(fields["output_parse_pii"]["type"], json!("bool"));
}

#[test]
fn projection_type_override_replaces_the_type_when_ui_type_is_absent() {
    let ratio = FormField {
        name: "ratio",
        description: None,
        required: false,
        shape: FormShape::Number,
        default: None,
        extra: FormExtra {
            ui_type: None,
            type_override: Some("percentage".to_owned()),
            options: None,
            min: None,
            max: None,
            step: None,
            default_value: None,
        },
    };
    let fields = project(vec![ratio]);
    assert_eq!(fields["ratio"]["type"], json!("percentage"));
}

#[test]
fn projection_ui_type_wins_over_type_override() {
    let flag = FormField {
        name: "flag",
        description: None,
        required: false,
        shape: FormShape::Boolean,
        default: None,
        extra: FormExtra {
            ui_type: Some("bool".to_owned()),
            type_override: Some("text".to_owned()),
            options: None,
            min: None,
            max: None,
            step: None,
            default_value: None,
        },
    };
    let fields = project(vec![flag]);
    assert_eq!(fields["flag"]["type"], json!("bool"));
}

#[test]
fn projection_ui_type_on_a_choice_skips_the_literal_options() {
    let mode = FormField {
        name: "mode",
        description: None,
        required: false,
        shape: FormShape::Choice(vec![json!("fast"), json!("slow")]),
        default: None,
        extra: FormExtra {
            ui_type: Some("text".to_owned()),
            type_override: None,
            options: None,
            min: None,
            max: None,
            step: None,
            default_value: None,
        },
    };
    let fields = project(vec![mode]);
    assert_eq!(fields["mode"]["type"], json!("text"));
    assert!(fields["mode"].get("options").is_none());
}

#[test]
fn projection_dict_field_gets_key_options_and_value_type() {
    let fields = project(vec![
        FormField {
            name: "weights",
            description: None,
            required: false,
            shape: FormShape::Mapping {
                keys: vec![json!("left"), json!("right")],
                value: Box::new(FormShape::Number),
            },
            default: None,
            extra: FormExtra::NONE,
        },
        FormField {
            name: "counts",
            description: None,
            required: false,
            shape: FormShape::Mapping {
                keys: Vec::new(),
                value: Box::new(FormShape::Number),
            },
            default: None,
            extra: FormExtra::NONE,
        },
    ]);
    assert_eq!(fields["weights"]["type"], json!("dict"));
    assert_eq!(
        fields["weights"]["dict_key_options"],
        json!(["left", "right"])
    );
    assert_eq!(fields["weights"]["dict_value_type"], json!("number"));
    assert!(fields["counts"].get("dict_key_options").is_none());
    assert_eq!(fields["counts"]["dict_value_type"], json!("number"));
}

#[test]
fn projection_type_override_to_dict_gets_a_string_value_type() {
    let text = FormField {
        name: "settings",
        description: None,
        required: false,
        shape: FormShape::Text,
        default: None,
        extra: FormExtra {
            ui_type: None,
            type_override: Some("dict".to_owned()),
            options: None,
            min: None,
            max: None,
            step: None,
            default_value: None,
        },
    };
    let fields = project(vec![text]);
    assert_eq!(fields["settings"]["type"], json!("dict"));
    assert_eq!(fields["settings"]["dict_value_type"], json!("string"));
    assert!(fields["settings"].get("dict_key_options").is_none());
}

#[test]
fn projection_list_of_literals_becomes_multiselect() {
    let fields = project(vec![field(
        "kinds",
        FormShape::List {
            options: vec![json!("alpha"), json!("beta")],
        },
    )]);
    assert_eq!(fields["kinds"]["type"], json!("multiselect"));
    assert_eq!(fields["kinds"]["options"], json!(["alpha", "beta"]));
}

#[test]
fn projection_list_without_literal_options_stays_an_array() {
    let fields = project(vec![field(
        "rules",
        FormShape::List {
            options: Vec::new(),
        },
    )]);
    assert_eq!(fields["rules"]["type"], json!("array"));
    assert!(fields["rules"].get("options").is_none());
}

#[test]
fn projection_select_gets_its_literal_values_as_options() {
    let fields = project(vec![field(
        "mode",
        FormShape::Choice(vec![json!("fast"), json!("slow")]),
    )]);
    assert_eq!(fields["mode"]["type"], json!("select"));
    assert_eq!(fields["mode"]["options"], json!(["fast", "slow"]));
}

#[test]
fn projection_select_without_values_has_no_options() {
    let fields = project(vec![field("mode", FormShape::Choice(Vec::new()))]);
    assert_eq!(fields["mode"]["type"], json!("select"));
    assert!(fields["mode"].get("options").is_none());
}

#[test]
fn projection_extra_options_win_over_derived_options() {
    let tagged = FormField {
        name: "tags",
        description: None,
        required: false,
        shape: FormShape::List {
            options: vec![json!("red")],
        },
        default: None,
        extra: FormExtra {
            ui_type: None,
            type_override: None,
            options: Some(vec![json!("blue"), json!("green")]),
            min: None,
            max: None,
            step: None,
            default_value: None,
        },
    };
    let choice = FormField {
        name: "mode",
        description: None,
        required: false,
        shape: FormShape::Choice(vec![json!("fast")]),
        default: None,
        extra: FormExtra {
            ui_type: None,
            type_override: None,
            options: Some(vec![json!("slow")]),
            min: None,
            max: None,
            step: None,
            default_value: None,
        },
    };
    let fields = project(vec![tagged, choice]);
    assert_eq!(fields["tags"]["type"], json!("multiselect"));
    assert_eq!(fields["tags"]["options"], json!(["blue", "green"]));
    assert_eq!(fields["mode"]["type"], json!("select"));
    assert_eq!(fields["mode"]["options"], json!(["slow"]));
}

#[test]
fn projection_default_value_is_kept_except_when_it_is_absent() {
    let disabled = FormField {
        name: "enabled",
        description: None,
        required: false,
        shape: FormShape::Boolean,
        default: Some(json!(false)),
        extra: FormExtra::NONE,
    };
    let absent = text_field("absent", None);
    let null = FormField {
        name: "empty",
        description: None,
        required: false,
        shape: FormShape::List {
            options: Vec::new(),
        },
        default: Some(Value::Null),
        extra: FormExtra::NONE,
    };
    let fields = project(vec![disabled, absent, null]);
    assert_eq!(fields["enabled"]["default_value"], json!(false));
    assert!(fields["absent"].get("default_value").is_none());
    assert_eq!(fields["empty"]["default_value"], json!(null));
}

#[test]
fn projection_extra_limits_and_default_value_overwrite_the_field() {
    let ratio = FormField {
        name: "ratio",
        description: None,
        required: false,
        shape: FormShape::Number,
        default: Some(json!(0.5_f64)),
        extra: FormExtra {
            ui_type: None,
            type_override: None,
            options: None,
            min: Some(json!(0.0_f64)),
            max: Some(json!(1.0_f64)),
            step: Some(json!(0.05_f64)),
            default_value: Some(json!(0.25_f64)),
        },
    };
    let fields = project(vec![ratio]);
    assert_eq!(fields["ratio"]["min"], json!(0.0_f64));
    assert_eq!(fields["ratio"]["max"], json!(1.0_f64));
    assert_eq!(fields["ratio"]["step"], json!(0.05_f64));
    assert_eq!(fields["ratio"]["default_value"], json!(0.25_f64));
}

#[test]
fn projection_depth_limit_allows_nesting_at_the_limit() {
    let fields = project_model(&chain(1), 1).unwrap();
    assert_eq!(fields["next"]["fields"]["leaf"]["type"], json!("string"));
}

#[test]
fn projection_depth_limit_rejects_one_level_more() {
    let error = project_model(&chain(1), 0).unwrap_err();
    assert!(matches!(error, FormError::TooDeep { limit: 0 }));
    let deeper = project_model(&chain(2), 1).unwrap_err();
    assert!(matches!(deeper, FormError::TooDeep { limit: 1 }));
}

#[test]
fn projection_flat_model_projects_at_depth_zero() {
    let fields = project_model(&chain(0), 0).unwrap();
    assert_eq!(fields["leaf"]["type"], json!("string"));
}

#[test]
fn projection_friendly_name_is_added_only_when_the_model_has_one() {
    let named = FormModel {
        ui_friendly_name: Some("Named Guardrail"),
        fields: Vec::new(),
    };
    let mut fields = project_model(&named, 0).unwrap();
    with_friendly_name(&named, &mut fields);
    assert_eq!(fields["ui_friendly_name"], json!("Named Guardrail"));
    let bare = model(Vec::new());
    let mut bare_fields = project_model(&bare, 0).unwrap();
    with_friendly_name(&bare, &mut bare_fields);
    assert!(bare_fields.get("ui_friendly_name").is_none());
}

fn projected_table(name: &str) -> Value {
    let tables = core_tables();
    let (_, model) = tables
        .iter()
        .find(|(key, _)| *key == name)
        .expect("the core tables hold every route-owned provider");
    let mut fields = project_model(model, 100).expect("the core tables nest below the limit");
    with_friendly_name(model, &mut fields);
    Value::Object(fields)
}

#[test]
fn core_table_bedrock_projects_like_litellm() {
    let expected: Value = serde_json::from_str(include_str!("fixtures/core-table-bedrock.json"))
        .expect("the expected form dictionary is JSON");
    assert_eq!(projected_table("bedrock"), expected);
}

#[test]
fn core_table_presidio_projects_like_litellm() {
    let expected: Value = serde_json::from_str(include_str!("fixtures/core-table-presidio.json"))
        .expect("the expected form dictionary is JSON");
    assert_eq!(projected_table("presidio"), expected);
}

#[test]
fn core_table_lakera_v2_projects_like_litellm() {
    let expected: Value = serde_json::from_str(include_str!("fixtures/core-table-lakera_v2.json"))
        .expect("the expected form dictionary is JSON");
    assert_eq!(projected_table("lakera_v2"), expected);
}

#[test]
fn core_table_tool_permission_projects_like_litellm() {
    let expected: Value =
        serde_json::from_str(include_str!("fixtures/core-table-tool_permission.json"))
            .expect("the expected form dictionary is JSON");
    assert_eq!(projected_table("tool_permission"), expected);
}

#[test]
fn core_table_hide_secrets_projects_like_litellm() {
    let expected: Value =
        serde_json::from_str(include_str!("fixtures/core-table-hide-secrets.json"))
            .expect("the expected form dictionary is JSON");
    assert_eq!(projected_table("hide-secrets"), expected);
}

#[test]
fn core_table_keys_are_the_route_names() {
    let keys: Vec<&str> = core_tables().iter().map(|(key, _)| *key).collect();
    assert_eq!(
        keys,
        [
            "bedrock",
            "presidio",
            "lakera_v2",
            "tool_permission",
            "hide-secrets"
        ]
    );
}
