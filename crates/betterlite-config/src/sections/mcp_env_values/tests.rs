#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

fn load(environment: &[(&str, &str)]) -> Result<super::PayloadRules, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }.load("")?.sections.mcp_env_values)
}

fn commands(names: &[&str]) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    for name in names {
        set.insert((*name).to_owned());
    }
    set
}

fn defaults() -> BTreeSet<String> {
    let base = ["npx", "uvx", "python", "python3", "node", "docker", "deno"];
    commands(&base)
}

#[test]
fn the_defaults_are_dash_and_stdio_disabled() {
    let rules = load(&[]).unwrap();
    assert_eq!(rules.prefix_separator, "-");
    assert!(!rules.stdio.enabled);
    assert_eq!(rules.stdio.allowed_commands, defaults());
}

#[test]
fn the_separator_is_read_with_an_empty_value_meaning_unset() {
    assert_eq!(
        load(&[("MCP_TOOL_PREFIX_SEPARATOR", ".")])
            .unwrap()
            .prefix_separator,
        "."
    );
    assert_eq!(
        load(&[("MCP_TOOL_PREFIX_SEPARATOR", "")])
            .unwrap()
            .prefix_separator,
        "-"
    );
    assert_eq!(
        load(&[("MCP_TOOL_PREFIX_SEPARATOR", "__")])
            .unwrap()
            .prefix_separator,
        "__"
    );
}

#[rstest]
#[case::lowercase_true("true", true)]
#[case::trimmed_uppercase_true(" TRUE ", true)]
#[case::capitalized_true("True", true)]
#[case::yes("yes", true)]
#[case::one("1", true)]
#[case::false_text("false", false)]
#[case::zero("0", false)]
#[case::empty("", false)]
fn stdio_flag_follows_the_shared_parser(#[case] value: &str, #[case] expected: bool) {
    let rules = load(&[("LITELLM_ENABLE_MCP_STDIO", value)]).unwrap();
    assert_eq!(rules.stdio.enabled, expected, "{value:?}");
}

#[test]
fn a_stdio_flag_that_is_not_a_bool_is_a_config_error() {
    assert!(load(&[("LITELLM_ENABLE_MCP_STDIO", "truex")]).is_err());
}

#[test]
fn extra_commands_add_to_the_base_set_and_empty_parts_drop() {
    let rules = load(&[("LITELLM_MCP_STDIO_EXTRA_COMMANDS", "foo,bar")]).unwrap();
    let mut expected = defaults();
    expected.insert("foo".to_owned());
    expected.insert("bar".to_owned());
    assert_eq!(rules.stdio.allowed_commands, expected);

    let with_gaps = load(&[("LITELLM_MCP_STDIO_EXTRA_COMMANDS", "foo,,bar,")]).unwrap();
    assert_eq!(with_gaps.stdio.allowed_commands, expected);

    let empty = load(&[("LITELLM_MCP_STDIO_EXTRA_COMMANDS", "")]).unwrap();
    assert_eq!(empty.stdio.allowed_commands, defaults());
}

#[test]
fn extra_commands_are_kept_as_written() {
    let rules = load(&[("LITELLM_MCP_STDIO_EXTRA_COMMANDS", " foo")]).unwrap();
    let mut expected = defaults();
    expected.insert(" foo".to_owned());
    assert_eq!(rules.stdio.allowed_commands, expected);
}

#[test]
fn the_three_variables_do_not_depend_on_the_yaml() {
    let rules = load(&[("LITELLM_ENABLE_MCP_STDIO", "true")]).unwrap();
    assert!(rules.stdio.enabled);
    assert_eq!(rules.stdio.allowed_commands, defaults());
}
