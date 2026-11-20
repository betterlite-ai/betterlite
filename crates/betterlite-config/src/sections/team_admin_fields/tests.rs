#![cfg(test)]

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs::read_to_string;
use std::sync::Arc;

use rstest::rstest;
use tempfile::NamedTempFile;
use tracing::subscriber::set_default;

use crate::loader::ConfigLoader;
use crate::sections::settings::JsonValue;
use crate::sections::team_admin_fields::{
    EDITABLE_FIELDS_SETTING, MEMBER_KEY_BUDGETS_PERMISSION, PROJECTS_PERMISSION, Resolution,
    SUPPORTED_TEAM_FIELDS, SettingWarning, may_edit_member_key_budgets, resolve_editable_fields,
};

fn loaded(source: &str) -> Option<JsonValue> {
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .unwrap();
    config.sections.team_admin_fields.editable_team_fields
}

fn resolved(source: &str, supported: &[&str]) -> (Resolution, String) {
    let log = NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    let guard = set_default(subscriber);
    let setting = loaded(source);
    let resolution = resolve_editable_fields(setting.as_ref(), supported);
    drop(guard);
    (resolution, read_to_string(log.path()).unwrap())
}

fn names(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[rstest]
#[case::absent("general_settings: {}")]
#[case::explicit_null("general_settings:\n  team_admin_editable_team_fields: null")]
#[case::empty_list("general_settings:\n  team_admin_editable_team_fields: []")]
fn absent_and_null_disable(#[case] source: &str) {
    let (resolution, log) = resolved(source, &SUPPORTED_TEAM_FIELDS);
    assert_eq!(resolution, Resolution::default());
    assert!(resolution.fields.is_empty());
    assert!(resolution.warnings.is_empty());
    assert_eq!(log, "");
}

#[test]
fn supported_subset_and_duplicates() {
    let source = "general_settings:\n  team_admin_editable_team_fields:\n    - rpm_limit\n    - rpm_limit\n    - max_budget\n    - member_key_budgets\n";
    let (resolution, log) = resolved(source, &SUPPORTED_TEAM_FIELDS);
    assert_eq!(resolution.fields, names(&["max_budget", "rpm_limit"]));
    assert!(resolution.warnings.is_empty());
    assert_eq!(log, "");
    assert!(may_edit_member_key_budgets(loaded(source).as_ref()));
    let expected = vec![
        "max_budget".to_owned(),
        "member_key_budgets".to_owned(),
        "projects".to_owned(),
        "rpm_limit".to_owned(),
        "tpm_limit".to_owned(),
    ];
    let (budgets, budgets_log) = resolved(source, &[MEMBER_KEY_BUDGETS_PERMISSION]);
    assert_eq!(budgets.fields, names(&[MEMBER_KEY_BUDGETS_PERMISSION]));
    assert!(budgets.warnings.is_empty());
    assert_eq!(budgets_log, "");
    let (projects, projects_log) = resolved(source, &[PROJECTS_PERMISSION]);
    assert!(projects.fields.is_empty());
    assert!(projects.warnings.is_empty());
    assert_eq!(projects_log, "");
    let unknown = "general_settings:\n  team_admin_editable_team_fields:\n    - max_budget\n    - member_key_budgets\n    - bogus\n";
    let (subset, _) = resolved(unknown, &[MEMBER_KEY_BUDGETS_PERMISSION]);
    assert_eq!(subset.fields, names(&[MEMBER_KEY_BUDGETS_PERMISSION]));
    let [
        SettingWarning::Unsupported {
            names: rejected,
            supported,
        },
    ] = subset.warnings.as_slice()
    else {
        panic!("only the unknown name is unsupported");
    };
    assert_eq!(rejected, &vec!["bogus".to_owned()]);
    assert_eq!(supported, &expected);
    let team_only = "general_settings:\n  team_admin_editable_team_fields:\n    - rpm_limit\n";
    assert!(!may_edit_member_key_budgets(loaded(team_only).as_ref()));
}

#[rstest]
#[case::scalar(
    "general_settings:\n  team_admin_editable_team_fields: max_budget",
    "max_budget"
)]
#[case::number_entry("general_settings:\n  team_admin_editable_team_fields:\n    - 3", "3")]
#[case::null_entry(
    "general_settings:\n  team_admin_editable_team_fields:\n    - null",
    "null"
)]
#[case::mapping(
    "general_settings:\n  team_admin_editable_team_fields:\n    max_budget: true",
    "max_budget"
)]
#[case::flag("general_settings:\n  team_admin_editable_team_fields: true", "true")]
fn malformed_list_warns_empty(#[case] source: &str, #[case] raw: &str) {
    let (resolution, log) = resolved(source, &SUPPORTED_TEAM_FIELDS);
    assert!(resolution.fields.is_empty());
    assert_eq!(
        resolution.warnings,
        vec![SettingWarning::NotAList {
            raw: loaded(source).unwrap()
        }]
    );
    assert!(log.contains(EDITABLE_FIELDS_SETTING), "{log}");
    assert!(log.contains("must be a list of field names"), "{log}");
    assert!(log.contains(raw), "{log}");
    assert!(resolution.warnings.first().unwrap().message().contains(raw));
}

#[test]
fn unsupported_sorted_warning() {
    let source = "general_settings:\n  team_admin_editable_team_fields:\n    - zeta_unknown\n    - alpha_unknown\n    - max_budget\n";
    let (resolution, log) = resolved(source, &SUPPORTED_TEAM_FIELDS);
    assert_eq!(resolution.fields, names(&["max_budget"]));
    let [
        SettingWarning::Unsupported {
            names: rejected,
            supported,
        },
    ] = resolution.warnings.as_slice()
    else {
        panic!("one unsupported warning names the rejected fields");
    };
    assert_eq!(
        rejected,
        &vec!["alpha_unknown".to_owned(), "zeta_unknown".to_owned()]
    );
    assert_eq!(
        supported,
        &vec![
            "max_budget".to_owned(),
            "member_key_budgets".to_owned(),
            "projects".to_owned(),
            "rpm_limit".to_owned(),
            "tpm_limit".to_owned(),
        ]
    );
    assert!(log.contains(EDITABLE_FIELDS_SETTING), "{log}");
    assert!(log.contains("ignores unsupported field(s)"), "{log}");
    let alpha = log.find("alpha_unknown").unwrap();
    let zeta = log.find("zeta_unknown").unwrap();
    assert!(alpha < zeta, "{log}");
}
