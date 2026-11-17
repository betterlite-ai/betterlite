#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::ConfigLoader;
use crate::sections::routing_groups::Settings;

fn settings(source: &str, variables: &[(&str, &str)]) -> Result<Settings, String> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.routing_groups)
        .map_err(|error| error.to_string())
}

#[test]
fn absent_and_null_groups_default_to_none() {
    assert_eq!(settings("model_list: []\n", &[]).unwrap(), None);
    assert_eq!(
        settings("router_settings:\n  routing_groups: null\n", &[]).unwrap(),
        None
    );
}

#[test]
fn an_explicit_empty_list_stays_as_given() {
    assert_eq!(
        settings("router_settings:\n  routing_groups: []\n", &[])
            .unwrap()
            .map(|loaded| loaded.groups),
        Some(Vec::new())
    );
}

#[test]
fn a_group_reads_its_fields_in_order() {
    let loaded = settings(
        "router_settings:\n  routing_groups:\n    - group_name: cheap\n      models: [a, b]\n      routing_strategy: least-busy\n      routing_strategy_args:\n        ttl: 5\n      model_priorities: {a: 1, b: 2}\n    - group_name: fast\n      models: [c]\n      routing_strategy: priority\n      model_priorities: {c: 9007199254740991}\n",
        &[],
    )
    .unwrap();
    let groups = loaded.unwrap().groups;
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].group_name, "cheap");
    assert_eq!(groups[0].models, vec![String::from("a"), String::from("b")]);
    assert_eq!(groups[0].routing_strategy, "least-busy");
    let args = groups[0].routing_strategy_args.as_ref().unwrap();
    assert_eq!(args.len(), 1);
    assert_eq!(
        serde_json::to_string(&args["ttl"]).unwrap(),
        "5",
        "the args map stays as written"
    );
    assert_eq!(
        groups[0].model_priorities,
        Some(BTreeMap::from([
            (String::from("a"), 1),
            (String::from("b"), 2)
        ]))
    );
    assert_eq!(
        groups[1].model_priorities,
        Some(BTreeMap::from([(String::from("c"), 0x001F_FFFF_FFFF_FFFF)]))
    );
}

#[test]
fn env_values_resolve_once_at_the_config_boundary() {
    let loaded = settings(
        "router_settings:\n  routing_groups:\n    - group_name: os.environ/GROUP\n      models: [os.environ/MODEL]\n      routing_strategy: os.environ/STRATEGY\n      model_priorities:\n        a: os.environ/PRIORITY\n",
        &[
            ("GROUP", "cheap"),
            ("MODEL", "a"),
            ("STRATEGY", "priority"),
            ("PRIORITY", "3"),
        ],
    )
    .unwrap();
    let loaded = loaded.unwrap();
    let group = &loaded.groups[0];
    assert_eq!(group.group_name, "cheap");
    assert_eq!(group.models, vec![String::from("a")]);
    assert_eq!(group.routing_strategy, "priority");
    assert_eq!(
        group.model_priorities,
        Some(BTreeMap::from([(String::from("a"), 3)]))
    );
}

#[test]
fn a_missing_environment_names_its_variable_and_path() {
    let error = settings(
        "router_settings:\n  routing_groups:\n    - group_name: cheap\n      models: [a]\n      routing_strategy: priority\n      model_priorities: {a: os.environ/PRIORITY}\n",
        &[],
    )
    .unwrap_err();
    assert!(error.contains("PRIORITY"), "{error}");
    assert!(
        error.contains("router_settings.routing_groups.0.model_priorities.a"),
        "{error}"
    );
}

#[test]
fn a_priority_that_is_not_an_integer_names_its_path() {
    let error = settings(
        "router_settings:\n  routing_groups:\n    - group_name: cheap\n      models: [a]\n      routing_strategy: priority\n      model_priorities: {a: first}\n",
        &[],
    )
    .unwrap_err();
    assert!(
        error.contains("router_settings.routing_groups.0.model_priorities.a"),
        "{error}"
    );
}

#[test]
fn a_group_without_a_strategy_fails_to_load() {
    let error = settings(
        "router_settings:\n  routing_groups:\n    - group_name: cheap\n      models: [a]\n",
        &[],
    )
    .unwrap_err();
    assert!(error.contains("routing_strategy"), "{error}");
}

#[test]
fn models_must_be_a_list_of_strings() {
    settings(
        "router_settings:\n  routing_groups:\n    - group_name: cheap\n      models: a\n      routing_strategy: simple-shuffle\n",
        &[],
    )
    .unwrap_err();
}

// the fields the router validates are carried verbatim, so a bad value names itself there
#[test]
fn a_group_carries_what_the_router_validates() {
    let mut groups = settings(
        "router_settings:\n  routing_groups:\n    - group_name: ''\n      models: [a]\n      routing_strategy: not-a-strategy\n",
        &[],
    )
    .unwrap()
    .unwrap()
    .groups;
    let group = groups.remove(0);
    assert!(group.group_name.is_empty());
    assert_eq!(group.routing_strategy, "not-a-strategy");
    assert_eq!(group.model_priorities, None);
    assert_eq!(group.routing_strategy_args, None);
}
