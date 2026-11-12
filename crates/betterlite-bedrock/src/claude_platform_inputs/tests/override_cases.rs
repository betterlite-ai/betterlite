use std::collections::BTreeSet;

use betterlite_config::sections::claude_platform_inputs::{PlatformValue, UnsupportedParams};
use indexmap::IndexMap;

use super::support;
use crate::claude_platform_inputs::body::{
    PlatformWarning, filter_request_body, resolve_unsupported_override,
};
use crate::model_routes::ParamValue;

fn set_of(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

pub(super) fn unsupported_override() {
    defaults_hold();
    sequences_collect_names();
    null_override_reads_as_absent();
    invalid_overrides_warn();
}

fn defaults_hold() {
    let missing = resolve_unsupported_override(None, None, true);
    assert_eq!(missing.names, None);
    assert_eq!(missing.warning, None);
}

fn null_override_reads_as_absent() {
    let deployment = support::unsupported(&["context_management"]);
    let null_override = UnsupportedParams::Other(PlatformValue::Null);
    let from_optional = resolve_unsupported_override(Some(&deployment), Some(&null_override), true);
    assert_eq!(from_optional.names, Some(set_of(&["context_management"])));
    assert_eq!(from_optional.warning, None);
}

fn sequences_collect_names() {
    let deployment = support::unsupported(&["context_management"]);
    let from_deployment = resolve_unsupported_override(Some(&deployment), None, true);
    assert_eq!(from_deployment.names, Some(set_of(&["context_management"])));
    assert_eq!(from_deployment.warning, None);

    let optional = support::unsupported(&["cache_control"]);
    let from_optional = resolve_unsupported_override(Some(&deployment), Some(&optional), true);
    assert_eq!(from_optional.names, Some(set_of(&["cache_control"])));

    let empty = UnsupportedParams::List(Vec::new());
    let empty_override = resolve_unsupported_override(Some(&deployment), Some(&empty), true);
    assert_eq!(empty_override.names, Some(BTreeSet::new()));
}

fn invalid_overrides_warn() {
    let deployment = support::unsupported(&["context_management"]);
    let invalid = UnsupportedParams::Other(PlatformValue::Text("not-a-list".to_owned()));
    let warned = resolve_unsupported_override(Some(&deployment), Some(&invalid), true);
    assert_eq!(warned.names, None);
    assert_eq!(
        warned.warning,
        Some(PlatformWarning::IgnoredOverride { type_name: "str" })
    );
    let warning = warned.warning.unwrap();
    assert_eq!(PlatformWarning::SUBJECT, "bedrock/claude_platform");
    assert!(
        warning
            .message()
            .starts_with("bedrock/claude_platform: ignoring")
    );
    assert!(warning.message().contains("of type str"));

    let silent = resolve_unsupported_override(Some(&deployment), Some(&invalid), false);
    assert_eq!(silent.names, None);
    assert_eq!(silent.warning, None);
}

pub(super) fn filtered_body_fields() {
    default_unsupported_is_dropped_and_warned();
    unrelated_keys_are_dropped_silently();
    override_sets_warn_in_map_order();
    override_names_replace_the_default();
}

fn default_unsupported_is_dropped_and_warned() {
    let empty = support::params(&[]);
    let untouched = filter_request_body(&empty, None, true);
    assert!(untouched.body.is_empty());
    assert_eq!(untouched.warning, None);

    let params = support::params(&[
        ("model", support::text("claude")),
        ("context_management", ParamValue::Null),
    ]);
    let filtered = filter_request_body(&params, None, true);
    assert_eq!(filtered.body.keys().collect::<Vec<_>>(), ["model"]);
    assert_eq!(
        filtered.warning,
        Some(PlatformWarning::DroppedUnsupported {
            params: vec!["context_management".to_owned()]
        })
    );
    let message = filtered.warning.unwrap().message();
    assert!(message.starts_with(
        "bedrock/claude_platform: dropping unsupported Messages API param(s) (\"context_management\")"
    ));
    let silent = filter_request_body(&params, None, false);
    assert_eq!(silent.warning, None);
    assert_eq!(silent.body.keys().collect::<Vec<_>>(), ["model"]);
}

fn unrelated_keys_are_dropped_silently() {
    let params = support::params(&[
        ("workspace_id", support::text("w")),
        ("aws_workspace_id", support::text("aw")),
        ("anthropic_workspace_id", support::text("u")),
        ("anthropic-workspace-id", support::text("h")),
        (
            "claude_platform_unsupported_params",
            ParamValue::List(Vec::new()),
        ),
        ("aws_region_name", support::text("us-west-2")),
        ("aws_access_key_id", support::text("key")),
        ("aws_custom", support::text("custom")),
        ("temperature", support::integer(0)),
        ("stream", ParamValue::Bool(false)),
        ("messages", ParamValue::List(Vec::new())),
    ]);
    let filtered = filter_request_body(&params, None, true);
    assert_eq!(
        filtered.body.keys().collect::<Vec<_>>(),
        ["temperature", "stream", "messages"]
    );
    assert_eq!(filtered.warning, None);
    assert_eq!(filtered.body.get("temperature"), Some(&support::integer(0)));
    assert_eq!(filtered.body.get("stream"), Some(&ParamValue::Bool(false)));
    assert_eq!(
        filtered.body.get("messages"),
        Some(&ParamValue::List(Vec::new()))
    );
}

fn override_sets_warn_in_map_order() {
    let ordered = support::params(&[
        ("z_key", support::text("z")),
        ("a_key", support::text("a")),
        ("kept", support::text("k")),
    ]);
    let ordered_override = set_of(&["a_key", "z_key"]);
    let in_order = filter_request_body(&ordered, Some(&ordered_override), true);
    assert_eq!(
        in_order.warning,
        Some(PlatformWarning::DroppedUnsupported {
            params: vec!["z_key".to_owned(), "a_key".to_owned()]
        })
    );
    assert_eq!(in_order.body.keys().collect::<Vec<_>>(), ["kept"]);

    let aws_override = set_of(&["aws_region_name"]);
    let aws_params = support::params(&[("aws_region_name", support::text("us-west-2"))]);
    let aws_filtered = filter_request_body(&aws_params, Some(&aws_override), true);
    assert_eq!(
        aws_filtered.warning,
        Some(PlatformWarning::DroppedUnsupported {
            params: vec!["aws_region_name".to_owned()]
        })
    );
}

fn override_names_replace_the_default() {
    let params = support::params(&[
        ("max_tokens", support::integer(16)),
        ("context_management", ParamValue::Null),
        ("stop", ParamValue::List(vec![support::text("end")])),
        ("thinking", ParamValue::Null),
    ]);
    let override_names = set_of(&["max_tokens", "stop"]);
    let filtered = filter_request_body(&params, Some(&override_names), true);
    assert_eq!(
        filtered.body.keys().collect::<Vec<_>>(),
        ["context_management", "thinking"]
    );
    assert_eq!(
        filtered.warning,
        Some(PlatformWarning::DroppedUnsupported {
            params: vec!["max_tokens".to_owned(), "stop".to_owned()]
        })
    );

    let empty_override = set_of(&[]);
    let kept = filter_request_body(&params, Some(&empty_override), true);
    assert!(kept.body.contains_key("context_management"));
    assert_eq!(kept.warning, None);

    let values = support::params(&[
        ("temperature", support::integer(0)),
        ("top_p", ParamValue::Null),
    ]);
    let preserved = filter_request_body(&values, None, true);
    assert_eq!(
        preserved.body.get("temperature"),
        Some(&support::integer(0))
    );
    assert_eq!(preserved.body.get("top_p"), Some(&ParamValue::Null));
}

#[test]
fn sequence_containers_all_collect() {
    let set = UnsupportedParams::Set(vec![
        support::platform_value("a"),
        support::platform_value("b"),
    ]);
    assert_eq!(
        resolve_unsupported_override(Some(&set), None, true).names,
        Some(set_of(&["a", "b"]))
    );
    let frozen = UnsupportedParams::FrozenSet(vec![support::platform_value("c")]);
    assert_eq!(
        resolve_unsupported_override(Some(&frozen), None, true).names,
        Some(set_of(&["c"]))
    );
    let tuple = UnsupportedParams::Tuple(vec![support::platform_value("d")]);
    assert_eq!(
        resolve_unsupported_override(Some(&tuple), None, true).names,
        Some(set_of(&["d"]))
    );
    let mixed = UnsupportedParams::List(vec![support::platform_integer(1), PlatformValue::Null]);
    assert_eq!(
        resolve_unsupported_override(Some(&mixed), None, true).names,
        Some(set_of(&["1", "None"]))
    );
}

#[test]
fn every_invalid_type_names_its_python_type() {
    let int_deployment = UnsupportedParams::Other(support::platform_integer(5));
    assert_eq!(
        resolve_unsupported_override(Some(&int_deployment), None, true)
            .warning
            .unwrap(),
        PlatformWarning::IgnoredOverride { type_name: "int" }
    );
    let float = UnsupportedParams::Other(PlatformValue::Number(
        serde_json::Number::from_f64(5.5).unwrap(),
    ));
    assert_eq!(
        resolve_unsupported_override(Some(&float), None, true)
            .warning
            .unwrap(),
        PlatformWarning::IgnoredOverride { type_name: "float" }
    );
    let boolean = UnsupportedParams::Other(PlatformValue::Boolean(true));
    assert_eq!(
        resolve_unsupported_override(Some(&boolean), None, true)
            .warning
            .unwrap(),
        PlatformWarning::IgnoredOverride { type_name: "bool" }
    );
    let mapping = UnsupportedParams::Other(PlatformValue::Mapping(IndexMap::from([(
        "a".to_owned(),
        support::platform_value("b"),
    )])));
    assert_eq!(
        resolve_unsupported_override(Some(&mapping), None, true)
            .warning
            .unwrap(),
        PlatformWarning::IgnoredOverride { type_name: "dict" }
    );
}
