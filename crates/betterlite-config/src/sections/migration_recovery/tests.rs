#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::migration_recovery::{
    DB_PUSH_ENV_VAR, REPLICA_IDENTITY_ENV_VAR, V2_RESOLVER_ENV_VAR,
};
use rstest::rstest;

use super::Settings;
use crate::loader::ConfigLoader;

fn load(pairs: &[(&str, &str)]) -> Settings {
    let lookup: BTreeMap<String, String> = pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load("model_list: []\n")
        .expect("config should load")
        .sections
        .migration_recovery
}

#[rstest]
#[case::unset(&[], (true, false, false))]
#[case::explicit_true(
    &[
        (V2_RESOLVER_ENV_VAR, "true"),
        (DB_PUSH_ENV_VAR, "true"),
        (REPLICA_IDENTITY_ENV_VAR, "true"),
    ],
    (true, true, true)
)]
#[case::explicit_false(
    &[
        (V2_RESOLVER_ENV_VAR, "false"),
        (DB_PUSH_ENV_VAR, "false"),
        (REPLICA_IDENTITY_ENV_VAR, "false"),
    ],
    (false, false, false)
)]
#[case::empty_v2_selector_disables_v2(
    &[(V2_RESOLVER_ENV_VAR, "")],
    (false, false, false)
)]
fn mode_env_defaults(#[case] variables: &[(&str, &str)], #[case] expected: (bool, bool, bool)) {
    let loaded = load(variables);
    assert_eq!(
        (
            loaded.modes.use_v2_migration_resolver,
            loaded.modes.use_prisma_db_push,
            loaded.set_replica_identity_full,
        ),
        expected
    );
}

#[rstest]
#[case::true_spelling("true", true)]
#[case::mixed_case_true("TRUE", true)]
#[case::one("1", true)]
#[case::t("t", true)]
#[case::y("y", true)]
#[case::yes("yes", true)]
#[case::false_spelling("false", false)]
#[case::zero("0", false)]
#[case::f("f", false)]
#[case::n("n", false)]
#[case::no("no", false)]
#[case::on_is_false("on", false)]
#[case::empty("", false)]
#[case::untrimmed_space(" yes", false)]
#[case::untrimmed_trailing("true ", false)]
#[case::unknown("2", false)]
fn extras_boolean_spellings(#[case] text: &str, #[case] expected: bool) {
    let push = load(&[(DB_PUSH_ENV_VAR, text)]);
    assert_eq!(push.modes.use_prisma_db_push, expected);
    let replica = load(&[(REPLICA_IDENTITY_ENV_VAR, text)]);
    assert_eq!(replica.set_replica_identity_full, expected);
    let resolver = load(&[(V2_RESOLVER_ENV_VAR, text)]);
    assert_eq!(resolver.modes.use_v2_migration_resolver, expected);
}
