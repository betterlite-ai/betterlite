use rstest::rstest;

use crate::error::FixtureError;
use crate::finops::FinOpsFixtureError;
use crate::finops::model::{
    ConfigName, ConfigRow, FinOpsSeed, KeyRow, OrganizationRow, UsageApiKey,
};
use crate::seed::FixtureSeed;

use super::support::{
    LIMIT, OVER_LIMIT, VANTAGE_SETTINGS, deterministic_rows, empty_finops, fixture_with,
    metadata_row, organization, sample_seed, secret, seed_with_config_rows, seed_with_daily_usage,
    seed_with_key_metadata, seed_with_organizations, usage_row,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExpectedError {
    Accepted,
    RedactedDiagnostics,
    TooManyRows {
        list: &'static str,
        count: usize,
        limit: usize,
    },
    DuplicateOrganization,
    MissingBudget,
    MissingUser(&'static str),
    UnknownKeyMetadata,
    DuplicateKeyMetadata,
    MissingOrganization(&'static str),
    KeyMetadataWithoutTeam,
    MissingTeam,
    ConflictingTeamOrganization,
    DuplicateDailyUsageId,
    MissingUsageUser,
    UnknownUsageKey,
    InvalidOrphanHash,
    InvalidTimestamp(&'static str),
    DuplicateUsageTuple,
    DuplicateConfigName,
}

fn error_matches(source: &FinOpsFixtureError, expected: ExpectedError) -> bool {
    match (source, expected) {
        (
            FinOpsFixtureError::TooManyRows { list, count, limit },
            ExpectedError::TooManyRows {
                list: wanted_list,
                count: wanted_count,
                limit: wanted_limit,
            },
        ) => *list == wanted_list && *count == wanted_count && *limit == wanted_limit,
        (
            FinOpsFixtureError::DuplicateOrganization { .. },
            ExpectedError::DuplicateOrganization,
        )
        | (FinOpsFixtureError::MissingBudget { .. }, ExpectedError::MissingBudget)
        | (FinOpsFixtureError::UnknownKeyMetadata { .. }, ExpectedError::UnknownKeyMetadata)
        | (FinOpsFixtureError::DuplicateKeyMetadata { .. }, ExpectedError::DuplicateKeyMetadata)
        | (
            FinOpsFixtureError::KeyMetadataWithoutTeam { .. },
            ExpectedError::KeyMetadataWithoutTeam,
        )
        | (FinOpsFixtureError::MissingTeam { .. }, ExpectedError::MissingTeam)
        | (
            FinOpsFixtureError::ConflictingTeamOrganization { .. },
            ExpectedError::ConflictingTeamOrganization,
        )
        | (
            FinOpsFixtureError::DuplicateDailyUsageId { .. },
            ExpectedError::DuplicateDailyUsageId,
        )
        | (FinOpsFixtureError::MissingUsageUser { .. }, ExpectedError::MissingUsageUser)
        | (FinOpsFixtureError::UnknownUsageKey { .. }, ExpectedError::UnknownUsageKey)
        | (FinOpsFixtureError::InvalidOrphanHash { .. }, ExpectedError::InvalidOrphanHash)
        | (FinOpsFixtureError::DuplicateUsageTuple { .. }, ExpectedError::DuplicateUsageTuple)
        | (FinOpsFixtureError::DuplicateConfigName { .. }, ExpectedError::DuplicateConfigName) => {
            true
        }
        (FinOpsFixtureError::MissingUser { field, .. }, ExpectedError::MissingUser(wanted)) => {
            *field == wanted
        }
        (
            FinOpsFixtureError::MissingOrganization { field, .. },
            ExpectedError::MissingOrganization(wanted),
        ) => *field == wanted,
        (
            FinOpsFixtureError::InvalidTimestamp { field, .. },
            ExpectedError::InvalidTimestamp(wanted),
        ) => *field == wanted,
        _ => false,
    }
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
    seed_with_key_metadata(vec![metadata_row(
        "sk-alice-token",
        None,
        Some("org-missing"),
        None,
    )])
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
    seed_with_key_metadata(
        (0..OVER_LIMIT)
            .map(|_| metadata_row("sk-alice-token", None, None, None))
            .collect(),
    )
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
    let mut fixture = fixture_with(empty_finops());
    fixture.keys = (0..LIMIT)
        .map(|index| KeyRow {
            token: secret(&format!("sk-key-{index}")),
            user_id: "alice".to_owned(),
            team_id: None,
            role: None,
        })
        .collect();
    let mut finops = empty_finops();
    finops.key_metadata = (0..LIMIT)
        .map(|index| metadata_row(&format!("sk-key-{index}"), None, None, None))
        .collect();
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
#[case::sample(sample_seed, ExpectedError::Accepted)]
#[case::empty_usage(empty_usage_seed, ExpectedError::Accepted)]
#[case::redacted_diagnostics(sample_seed, ExpectedError::RedactedDiagnostics)]
#[case::null_tuple(null_tuple_seed, ExpectedError::Accepted)]
#[case::too_many_organizations(
    too_many_organizations_seed,
    ExpectedError::TooManyRows { list: "organizations", count: OVER_LIMIT, limit: LIMIT }
)]
#[case::too_many_key_metadata(
    too_many_key_metadata_seed,
    ExpectedError::TooManyRows { list: "key_metadata", count: OVER_LIMIT, limit: LIMIT }
)]
#[case::too_many_daily_usage(
    too_many_daily_usage_seed,
    ExpectedError::TooManyRows { list: "daily_usage", count: OVER_LIMIT, limit: LIMIT }
)]
#[case::too_many_config_rows(
    too_many_config_rows_seed,
    ExpectedError::TooManyRows { list: "config_rows", count: OVER_LIMIT, limit: LIMIT }
)]
#[case::boundary_organizations(boundary_organizations_seed, ExpectedError::Accepted)]
#[case::boundary_key_metadata(boundary_key_metadata_seed, ExpectedError::Accepted)]
#[case::boundary_daily_usage(boundary_daily_usage_seed, ExpectedError::Accepted)]
#[case::duplicate_organization(duplicate_organization_seed, ExpectedError::DuplicateOrganization)]
#[case::missing_budget(missing_budget_seed, ExpectedError::MissingBudget)]
#[case::missing_creator(missing_creator_seed, ExpectedError::MissingUser("created_by"))]
#[case::missing_updater(missing_updater_seed, ExpectedError::MissingUser("updated_by"))]
#[case::unknown_metadata_key(unknown_metadata_key_seed, ExpectedError::UnknownKeyMetadata)]
#[case::duplicate_metadata(duplicate_metadata_seed, ExpectedError::DuplicateKeyMetadata)]
#[case::missing_metadata_organization(
    missing_metadata_organization_seed,
    ExpectedError::MissingOrganization("organization_id")
)]
#[case::missing_team_organization(
    missing_team_organization_seed,
    ExpectedError::MissingOrganization("team_organization_id")
)]
#[case::key_without_team(key_without_team_seed, ExpectedError::KeyMetadataWithoutTeam)]
#[case::missing_team(missing_team_seed, ExpectedError::MissingTeam)]
#[case::conflicting_team(conflicting_team_seed, ExpectedError::ConflictingTeamOrganization)]
#[case::duplicate_usage_id(duplicate_usage_id_seed, ExpectedError::DuplicateDailyUsageId)]
#[case::missing_usage_user(missing_usage_user_seed, ExpectedError::MissingUsageUser)]
#[case::unknown_usage_key(unknown_usage_key_seed, ExpectedError::UnknownUsageKey)]
#[case::invalid_orphan_hash(invalid_orphan_hash_seed, ExpectedError::InvalidOrphanHash)]
#[case::invalid_created_at(invalid_created_at_seed, ExpectedError::InvalidTimestamp("created_at"))]
#[case::invalid_updated_at(invalid_updated_at_seed, ExpectedError::InvalidTimestamp("updated_at"))]
#[case::duplicate_tuple(duplicate_tuple_seed, ExpectedError::DuplicateUsageTuple)]
#[case::duplicate_config_name(duplicate_config_name_seed, ExpectedError::DuplicateConfigName)]
fn finops_seed_validation_contract(
    #[case] build: fn() -> FixtureSeed,
    #[case] expected: ExpectedError,
) {
    let fixture = build();
    if expected == ExpectedError::RedactedDiagnostics {
        let finops: &FinOpsSeed = fixture.finops.as_ref().expect("the seed carries finops");
        let debug = format!("{finops:?}");
        assert!(!debug.contains("sk-alice-token"));
        assert!(!debug.contains(VANTAGE_SETTINGS));
        assert!(debug.contains("[REDACTED]"));
        let rejected = unknown_metadata_key_seed();
        let error = rejected.validate().expect_err("the seed is rejected");
        assert!(!format!("{error}").contains("sk-missing"));
        assert!(!format!("{error:?}").contains("sk-missing"));
        return;
    }
    if expected == ExpectedError::MissingTeam {
        let finops = fixture.finops.as_ref().expect("the seed carries finops");
        let error = finops.validate(&fixture).expect_err("the seed is rejected");
        assert!(
            error_matches(&error, expected),
            "unexpected finops error {error:?}"
        );
        return;
    }
    if expected == ExpectedError::Accepted {
        fixture.validate().expect("the finops seed validates");
        return;
    }
    let error = fixture.validate().expect_err("the finops seed is rejected");
    let FixtureError::FinOps(source) = error else {
        panic!("expected a finops fixture error, got {error:?}");
    };
    assert!(
        error_matches(&source, expected),
        "unexpected finops error {source:?}"
    );
}
