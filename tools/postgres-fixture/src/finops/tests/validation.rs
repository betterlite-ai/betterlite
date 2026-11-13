use rstest::rstest;

use crate::error::FixtureError;
use crate::finops::FinOpsSeed;
use crate::finops::model::{ConfigName, ConfigRow, OrganizationRow, UsageApiKey};
use crate::seed::{FixtureSeed, KeyRow};

use super::support::{
    LIMIT, ORPHAN_HASH, OVER_LIMIT, VANTAGE_SETTINGS, deterministic_rows, fixture_with,
    metadata_row, organization, sample_finops, sample_seed, secret, seed_with_config_rows,
    seed_with_daily_usage, seed_with_key_metadata, seed_with_organizations, usage_row,
};

enum Expected {
    Accepted,
    Redacted,
    Direct(&'static str),
    Fixture(&'static str),
    Equality,
}

fn assert_changes(sample: &FinOpsSeed, change: fn(&mut FinOpsSeed)) {
    let mut changed = sample.clone();
    change(&mut changed);
    assert_ne!(sample, &changed);
}

fn replace_key_token(seed: &mut FinOpsSeed) {
    if let Some(row) = seed.key_metadata.first_mut() {
        row.token = secret("sk-other");
    }
}

fn replace_key_alias(seed: &mut FinOpsSeed) {
    if let Some(row) = seed.key_metadata.first_mut() {
        row.key_alias = Some("other-key".to_owned());
    }
}

fn clear_key_organization(seed: &mut FinOpsSeed) {
    if let Some(row) = seed.key_metadata.first_mut() {
        row.organization_id = None;
    }
}

fn clear_team_organization(seed: &mut FinOpsSeed) {
    if let Some(row) = seed.key_metadata.first_mut() {
        row.team_organization_id = None;
    }
}

fn rename_config(seed: &mut FinOpsSeed) {
    if let Some(row) = seed.config_rows.first_mut() {
        row.name = ConfigName::CloudzeroSettings;
    }
}

fn clear_config_value(seed: &mut FinOpsSeed) {
    if let Some(row) = seed.config_rows.first_mut() {
        row.serialized_value = None;
    }
}

fn replace_config_value(seed: &mut FinOpsSeed) {
    if let Some(row) = seed.config_rows.first_mut() {
        row.serialized_value = Some(secret("other"));
    }
}

fn replace_usage_key(seed: &mut FinOpsSeed) {
    if let Some(row) = seed.daily_usage.first_mut() {
        row.api_key = UsageApiKey::Key(secret("sk-other"));
    }
}

fn orphan_usage_key(seed: &mut FinOpsSeed) {
    if let Some(row) = seed.daily_usage.first_mut() {
        row.api_key = UsageApiKey::OrphanHash(ORPHAN_HASH.to_owned());
    }
}

fn assert_finops_equality() {
    let sample = sample_finops();
    assert_eq!(sample, sample_finops());
    assert_changes(&sample, replace_key_token);
    assert_changes(&sample, replace_key_alias);
    assert_changes(&sample, clear_key_organization);
    assert_changes(&sample, clear_team_organization);
    assert_changes(&sample, rename_config);
    assert_changes(&sample, clear_config_value);
    assert_changes(&sample, replace_config_value);
    assert_changes(&sample, replace_usage_key);
    assert_changes(&sample, orphan_usage_key);
    assert_eq!(ConfigName::VantageSettings.as_wire(), "vantage_settings");
    assert_eq!(
        ConfigName::CloudzeroSettings.as_wire(),
        "cloudzero_settings"
    );
}

fn assert_redacted(fixture: &FixtureSeed) {
    let finops = fixture.finops.as_ref().expect("the seed carries finops");
    let debug = format!("{finops:?}");
    assert!(!debug.contains("sk-alice-token"));
    assert!(!debug.contains(VANTAGE_SETTINGS));
    assert!(debug.contains("[REDACTED]"));
    let rejected = unknown_metadata_key_seed();
    let error = rejected.validate().expect_err("the seed is rejected");
    assert!(!format!("{error}").contains("sk-missing"));
    assert!(!format!("{error:?}").contains("sk-missing"));
}

fn empty_usage_seed() -> FixtureSeed {
    seed_with_daily_usage(Vec::new())
}

fn duplicate_organization_seed() -> FixtureSeed {
    seed_with_organizations(vec![
        organization("org-key", "Key Org"),
        organization("org-key", "Key Org Again"),
    ])
}

fn missing_budget_seed() -> FixtureSeed {
    let mut organization = organization("org-key", "Key Org");
    organization.budget_id = "budget-missing".to_owned();
    seed_with_organizations(vec![organization])
}

fn missing_creator_seed() -> FixtureSeed {
    let mut organization = organization("org-key", "Key Org");
    organization.created_by = "carol".to_owned();
    seed_with_organizations(vec![organization])
}

fn missing_updater_seed() -> FixtureSeed {
    let mut organization = organization("org-key", "Key Org");
    organization.updated_by = "carol".to_owned();
    seed_with_organizations(vec![organization])
}

fn unknown_metadata_key_seed() -> FixtureSeed {
    seed_with_key_metadata(vec![metadata_row("sk-missing", None, None, None)])
}

fn duplicate_metadata_seed() -> FixtureSeed {
    seed_with_key_metadata(vec![
        metadata_row("sk-alice-token", None, None, None),
        metadata_row("sk-alice-token", Some("alice-key"), None, None),
    ])
}

fn missing_metadata_organization_seed() -> FixtureSeed {
    let row = metadata_row("sk-alice-token", None, Some("org-missing"), None);
    seed_with_key_metadata(vec![row])
}

fn missing_team_organization_seed() -> FixtureSeed {
    seed_with_key_metadata(vec![metadata_row(
        "sk-alice-token",
        None,
        None,
        Some("org-missing"),
    )])
}

fn key_without_team_seed() -> FixtureSeed {
    seed_with_key_metadata(vec![metadata_row(
        "sk-teamless-token",
        None,
        None,
        Some("org-team"),
    )])
}

fn missing_team_seed() -> FixtureSeed {
    let mut fixture = seed_with_key_metadata(vec![metadata_row(
        "sk-alice-token",
        None,
        None,
        Some("org-team"),
    )]);
    let Some(key) = fixture.keys.first_mut() else {
        panic!("the fixture has a key");
    };
    key.team_id = Some("team-missing".to_owned());
    fixture
}

fn conflicting_team_seed() -> FixtureSeed {
    seed_with_key_metadata(vec![
        metadata_row("sk-alice-token", None, None, Some("org-team")),
        metadata_row("sk-second-red", None, None, Some("org-other")),
    ])
}

fn matching_team_seed() -> FixtureSeed {
    seed_with_key_metadata(vec![
        metadata_row("sk-alice-token", None, None, Some("org-team")),
        metadata_row("sk-second-red", None, None, Some("org-team")),
    ])
}

fn invalid_orphan_chars_seed() -> FixtureSeed {
    seed_with_daily_usage(vec![usage_row(
        "row-orphan",
        UsageApiKey::OrphanHash("z".repeat(64)),
    )])
}

fn duplicate_usage_id_seed() -> FixtureSeed {
    seed_with_daily_usage(vec![
        usage_row("row-same", UsageApiKey::Key(secret("sk-alice-token"))),
        usage_row("row-same", UsageApiKey::Key(secret("sk-bob-token"))),
    ])
}

fn missing_usage_user_seed() -> FixtureSeed {
    let mut row = usage_row("row-user", UsageApiKey::Key(secret("sk-alice-token")));
    row.user_id = Some("carol".to_owned());
    seed_with_daily_usage(vec![row])
}

fn unknown_usage_key_seed() -> FixtureSeed {
    seed_with_daily_usage(vec![usage_row(
        "row-key",
        UsageApiKey::Key(secret("sk-missing")),
    )])
}

fn invalid_orphan_hash_seed() -> FixtureSeed {
    seed_with_daily_usage(vec![usage_row(
        "row-orphan",
        UsageApiKey::OrphanHash("abc".to_owned()),
    )])
}

fn invalid_created_at_seed() -> FixtureSeed {
    let mut row = usage_row("row-time", UsageApiKey::Key(secret("sk-alice-token")));
    row.created_at = "not-a-timestamp".to_owned();
    seed_with_daily_usage(vec![row])
}

fn invalid_updated_at_seed() -> FixtureSeed {
    let mut row = usage_row("row-time", UsageApiKey::Key(secret("sk-alice-token")));
    row.updated_at = "2025-13-01T00:00:00".to_owned();
    seed_with_daily_usage(vec![row])
}

fn non_utc_offset_seed() -> FixtureSeed {
    let mut row = usage_row("row-time", UsageApiKey::Key(secret("sk-alice-token")));
    row.created_at = "2025-01-01T23:30:00-02:00".to_owned();
    seed_with_daily_usage(vec![row])
}

fn utc_offset_seed() -> FixtureSeed {
    let mut row = usage_row("row-time", UsageApiKey::Key(secret("sk-alice-token")));
    row.created_at = "2025-01-01T00:00:00Z".to_owned();
    row.updated_at = "2025-01-02T00:00:00+00:00".to_owned();
    seed_with_daily_usage(vec![row])
}

fn duplicate_tuple_seed() -> FixtureSeed {
    let first = usage_row("row-first", UsageApiKey::Key(secret("sk-alice-token")));
    let mut second = first.clone();
    second.id = "row-second".to_owned();
    seed_with_daily_usage(vec![first, second])
}

fn null_tuple_seed() -> FixtureSeed {
    let mut first = usage_row("row-first", UsageApiKey::Key(secret("sk-alice-token")));
    let mut second = usage_row("row-second", UsageApiKey::Key(secret("sk-alice-token")));
    first.model = None;
    second.model = None;
    seed_with_daily_usage(vec![first, second])
}

fn duplicate_config_name_seed() -> FixtureSeed {
    seed_with_config_rows(vec![
        ConfigRow {
            name: ConfigName::VantageSettings,
            serialized_value: None,
        },
        ConfigRow {
            name: ConfigName::VantageSettings,
            serialized_value: Some(secret("second")),
        },
    ])
}

fn repeated_organizations(count: usize) -> Vec<OrganizationRow> {
    (0..count)
        .map(|index| organization(&format!("org-{index}"), "Org"))
        .collect()
}

fn too_many_organizations_seed() -> FixtureSeed {
    seed_with_organizations(repeated_organizations(OVER_LIMIT))
}

fn too_many_key_metadata_seed() -> FixtureSeed {
    seed_with_key_metadata(vec![
        metadata_row("sk-alice-token", None, None, None);
        OVER_LIMIT
    ])
}

fn too_many_daily_usage_seed() -> FixtureSeed {
    seed_with_daily_usage(deterministic_rows(OVER_LIMIT))
}

fn too_many_config_rows_seed() -> FixtureSeed {
    seed_with_config_rows(vec![
        ConfigRow {
            name: ConfigName::VantageSettings,
            serialized_value: None,
        };
        OVER_LIMIT
    ])
}

fn boundary_organizations_seed() -> FixtureSeed {
    seed_with_organizations(repeated_organizations(LIMIT))
}

fn boundary_key_metadata_seed() -> FixtureSeed {
    let mut fixture = fixture_with(FinOpsSeed::default());
    fixture.keys = (0..LIMIT)
        .map(|index| KeyRow {
            token: secret(&format!("sk-key-{index}")),
            user_id: "alice".to_owned(),
            team_id: None,
            role: None,
        })
        .collect();
    let finops = FinOpsSeed {
        organizations: Vec::new(),
        key_metadata: (0..LIMIT)
            .map(|index| metadata_row(&format!("sk-key-{index}"), None, None, None))
            .collect(),
        daily_usage: Vec::new(),
        config_rows: Vec::new(),
    };
    fixture.finops = Some(finops);
    fixture
}

fn boundary_daily_usage_seed() -> FixtureSeed {
    let rows = (0..LIMIT)
        .map(|index| {
            let mut row = usage_row(
                &format!("row-{index}"),
                UsageApiKey::Key(secret("sk-alice-token")),
            );
            row.endpoint = Some(format!("/v1/{index}"));
            row
        })
        .collect();
    seed_with_daily_usage(rows)
}

#[rstest]
#[case::sample(sample_seed, Expected::Accepted)]
#[case::empty_usage(empty_usage_seed, Expected::Accepted)]
#[case::redacted_diagnostics(sample_seed, Expected::Redacted)]
#[case::equality(sample_seed, Expected::Equality)]
#[case::null_tuple(null_tuple_seed, Expected::Accepted)]
#[case::too_many_organizations(
    too_many_organizations_seed,
    Expected::Fixture("finops fixture organizations has 1025 rows, more than the limit of 1024")
)]
#[case::too_many_key_metadata(
    too_many_key_metadata_seed,
    Expected::Fixture("finops fixture key_metadata has 1025 rows, more than the limit of 1024")
)]
#[case::too_many_daily_usage(
    too_many_daily_usage_seed,
    Expected::Fixture("finops fixture daily_usage has 1025 rows, more than the limit of 1024")
)]
#[case::too_many_config_rows(
    too_many_config_rows_seed,
    Expected::Fixture("finops fixture config_rows has 1025 rows, more than the limit of 1024")
)]
#[case::boundary_organizations(boundary_organizations_seed, Expected::Accepted)]
#[case::boundary_key_metadata(boundary_key_metadata_seed, Expected::Accepted)]
#[case::boundary_daily_usage(boundary_daily_usage_seed, Expected::Accepted)]
#[case::duplicate_organization(
    duplicate_organization_seed,
    Expected::Fixture("finops organization row 1 repeats an organization_id")
)]
#[case::missing_budget(
    missing_budget_seed,
    Expected::Fixture("finops organization row 0 references missing budget budget-missing")
)]
#[case::missing_creator(
    missing_creator_seed,
    Expected::Fixture("finops organization row 0 field created_by references missing user carol")
)]
#[case::missing_updater(
    missing_updater_seed,
    Expected::Fixture("finops organization row 0 field updated_by references missing user carol")
)]
#[case::unknown_metadata_key(
    unknown_metadata_key_seed,
    Expected::Fixture("finops key metadata row 0 references no fixture key")
)]
#[case::duplicate_metadata(
    duplicate_metadata_seed,
    Expected::Fixture("finops key metadata row 1 repeats an earlier key target")
)]
#[case::missing_metadata_organization(
    missing_metadata_organization_seed,
    Expected::Fixture(
        "finops key metadata row 0 field organization_id references missing organization org-missing"
    )
)]
#[case::missing_team_organization(
    missing_team_organization_seed,
    Expected::Fixture(
        "finops key metadata row 0 field team_organization_id references missing organization org-missing"
    )
)]
#[case::key_without_team(
    key_without_team_seed,
    Expected::Fixture(
        "finops key metadata row 0 sets team_organization_id but its key has no team"
    )
)]
#[case::missing_team(
    missing_team_seed,
    Expected::Direct("finops key metadata row 0 references missing team team-missing")
)]
#[case::conflicting_team(
    conflicting_team_seed,
    Expected::Fixture("finops key metadata rows assign team team-red conflicting organizations")
)]
#[case::matching_team(matching_team_seed, Expected::Accepted)]
#[case::duplicate_usage_id(
    duplicate_usage_id_seed,
    Expected::Fixture("finops daily usage row 1 repeats an id")
)]
#[case::missing_usage_user(
    missing_usage_user_seed,
    Expected::Fixture("finops daily usage row 0 references missing user carol")
)]
#[case::unknown_usage_key(
    unknown_usage_key_seed,
    Expected::Fixture("finops daily usage row 0 references no fixture key")
)]
#[case::invalid_orphan_hash(
    invalid_orphan_hash_seed,
    Expected::Fixture("finops daily usage row 0 orphan_hash is not a SHA256 hex token")
)]
#[case::invalid_orphan_chars(
    invalid_orphan_chars_seed,
    Expected::Fixture("finops daily usage row 0 orphan_hash is not a SHA256 hex token")
)]
#[case::invalid_created_at(
    invalid_created_at_seed,
    Expected::Fixture("finops daily usage row 0 field created_at is not a valid timestamp")
)]
#[case::invalid_updated_at(
    invalid_updated_at_seed,
    Expected::Fixture("finops daily usage row 0 field updated_at is not a valid timestamp")
)]
#[case::non_utc_offset(
    non_utc_offset_seed,
    Expected::Fixture("finops daily usage row 0 field created_at is not a valid timestamp")
)]
#[case::utc_offset(utc_offset_seed, Expected::Accepted)]
#[case::duplicate_tuple(
    duplicate_tuple_seed,
    Expected::Fixture("finops daily usage row 1 repeats a non-null unique tuple")
)]
#[case::duplicate_config_name(
    duplicate_config_name_seed,
    Expected::Fixture("finops config row 1 repeats config name VantageSettings")
)]
fn finops_seed_validation_contract(#[case] build: fn() -> FixtureSeed, #[case] expected: Expected) {
    let fixture = build();
    match expected {
        Expected::Accepted => fixture.validate().expect("the finops seed validates"),
        Expected::Redacted => assert_redacted(&fixture),
        Expected::Equality => assert_finops_equality(),
        Expected::Direct(message) => {
            let finops = fixture.finops.as_ref().expect("the seed carries finops");
            let error = finops.validate(&fixture).expect_err("the seed is rejected");
            assert_eq!(error.to_string(), message);
        }
        Expected::Fixture(message) => {
            let error = fixture.validate().expect_err("the finops seed is rejected");
            let FixtureError::FinOps(source) = error else {
                panic!("expected a finops fixture error, got {error:?}");
            };
            assert_eq!(source.to_string(), message);
        }
    }
}
