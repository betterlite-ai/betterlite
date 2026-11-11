use std::cmp::Reverse;

use rust_decimal::Decimal;
use secrecy::SecretString;

use crate::finops::FinOpsSeed;
use crate::finops::model::{
    ConfigName, ConfigRow, FinOpsUsageRow, KeyMetadataRow, OrganizationRow, UsageApiKey,
};
use crate::seed::{BudgetRow, CallerRole, FixtureSeed, KeyRow, TeamRow, UserRow};

pub(super) const LIMIT: usize = 1024;
pub(super) const OVER_LIMIT: usize = 1025;
pub(super) const ORPHAN_HASH: &str =
    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
pub(super) const VANTAGE_SETTINGS: &str = "{\"api_key\":\"ciphertext-not-a-key\"}";
const DATES: [&str; 3] = ["2025-01-01", "2025-01-02", "2025-01-03"];

pub(super) fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn fixture_users() -> Vec<UserRow> {
    vec![
        UserRow {
            user_id: "alice".to_owned(),
            user_email: None,
        },
        UserRow {
            user_id: "bob".to_owned(),
            user_email: None,
        },
    ]
}

fn fixture_keys() -> Vec<KeyRow> {
    vec![
        KeyRow {
            token: secret("sk-alice-token"),
            user_id: "alice".to_owned(),
            team_id: Some("team-red".to_owned()),
            role: Some(CallerRole::ProxyAdmin),
        },
        KeyRow {
            token: secret("sk-second-red"),
            user_id: "alice".to_owned(),
            team_id: Some("team-red".to_owned()),
            role: None,
        },
        KeyRow {
            token: secret("sk-bob-token"),
            user_id: "bob".to_owned(),
            team_id: Some("team-blue".to_owned()),
            role: None,
        },
        KeyRow {
            token: secret("sk-teamless-token"),
            user_id: "bob".to_owned(),
            team_id: None,
            role: None,
        },
    ]
}

fn fixture_teams() -> Vec<TeamRow> {
    vec![
        TeamRow {
            team_id: "team-red".to_owned(),
            team_alias: Some("Red Team".to_owned()),
        },
        TeamRow {
            team_id: "team-blue".to_owned(),
            team_alias: Some("Blue Team".to_owned()),
        },
    ]
}

fn fixture_budgets() -> Vec<BudgetRow> {
    vec![BudgetRow {
        budget_id: "budget-1".to_owned(),
        user_id: "alice".to_owned(),
        max_budget: Some(Decimal::new(1000, 2)),
    }]
}

fn fill_fixture(seed: &mut FixtureSeed, finops: FinOpsSeed) {
    seed.users = fixture_users();
    seed.keys = fixture_keys();
    seed.teams = fixture_teams();
    seed.budgets = fixture_budgets();
    seed.finops = Some(finops);
}

pub(super) fn fixture_with(finops: FinOpsSeed) -> FixtureSeed {
    let mut seed = FixtureSeed::default();
    fill_fixture(&mut seed, finops);
    seed
}

pub(super) fn organization(id: &str, alias: &str) -> OrganizationRow {
    OrganizationRow {
        organization_id: id.to_owned(),
        organization_alias: alias.to_owned(),
        budget_id: "budget-1".to_owned(),
        created_by: "alice".to_owned(),
        updated_by: "alice".to_owned(),
        models: Vec::new(),
    }
}

pub(super) fn metadata_row(
    token: &str,
    key_alias: Option<&str>,
    organization_id: Option<&str>,
    team_organization_id: Option<&str>,
) -> KeyMetadataRow {
    KeyMetadataRow {
        token: secret(token),
        key_alias: key_alias.map(str::to_owned),
        organization_id: organization_id.map(str::to_owned),
        team_organization_id: team_organization_id.map(str::to_owned),
    }
}

pub(super) fn usage_row(id: &str, api_key: UsageApiKey) -> FinOpsUsageRow {
    FinOpsUsageRow {
        id: id.to_owned(),
        user_id: Some("alice".to_owned()),
        date: "2025-01-01".to_owned(),
        api_key,
        model: Some("openai/gpt-5".to_owned()),
        model_group: Some("default".to_owned()),
        custom_llm_provider: Some("openai".to_owned()),
        mcp_namespaced_tool_name: Some("mcp__tool".to_owned()),
        endpoint: Some("/v1/chat/completions".to_owned()),
        prompt_tokens: 10,
        completion_tokens: 5,
        cache_read_input_tokens: 3,
        cache_creation_input_tokens: 4,
        compression_saved_tokens: 2,
        compression_savings_spend: Decimal::new(5, 1),
        prompt_caching_savings_spend: Decimal::new(25, 2),
        gateway_injected_savings: Decimal::new(125, 3),
        autorouter_savings_spend: Decimal::new(625, 4),
        spend: Decimal::new(125, 2),
        api_requests: 2,
        successful_requests: 1,
        failed_requests: 1,
        total_response_time_ms: 120,
        timed_requests: 2,
        created_at: "2025-01-01T00:00:00".to_owned(),
        updated_at: "2025-01-01T00:00:05".to_owned(),
    }
}

fn orphan_row() -> FinOpsUsageRow {
    let mut orphan = usage_row(
        "row-orphan",
        UsageApiKey::OrphanHash(ORPHAN_HASH.to_owned()),
    );
    orphan.user_id = None;
    orphan.model = None;
    orphan.model_group = None;
    orphan.custom_llm_provider = None;
    orphan.mcp_namespaced_tool_name = None;
    orphan.endpoint = None;
    orphan
}

fn sample_organizations() -> Vec<OrganizationRow> {
    vec![
        organization("org-key", "Key Org"),
        organization("org-team", "Team Org"),
        organization("org-other", "Other Org"),
    ]
}

fn sample_metadata() -> Vec<KeyMetadataRow> {
    vec![
        metadata_row(
            "sk-alice-token",
            Some("alice-key"),
            Some("org-key"),
            Some("org-team"),
        ),
        metadata_row("sk-bob-token", None, None, Some("org-other")),
    ]
}

fn sample_config_rows() -> Vec<ConfigRow> {
    vec![
        ConfigRow {
            name: ConfigName::VantageSettings,
            serialized_value: Some(secret(VANTAGE_SETTINGS)),
        },
        ConfigRow {
            name: ConfigName::CloudzeroSettings,
            serialized_value: None,
        },
    ]
}

fn with_lists(
    organizations: Vec<OrganizationRow>,
    key_metadata: Vec<KeyMetadataRow>,
    daily_usage: Vec<FinOpsUsageRow>,
    config_rows: Vec<ConfigRow>,
) -> FinOpsSeed {
    FinOpsSeed {
        organizations,
        key_metadata,
        daily_usage,
        config_rows,
    }
}

pub(super) fn sample_finops() -> FinOpsSeed {
    with_lists(
        sample_organizations(),
        sample_metadata(),
        vec![
            usage_row("row-key", UsageApiKey::Key(secret("sk-alice-token"))),
            usage_row("row-team", UsageApiKey::Key(secret("sk-bob-token"))),
            orphan_row(),
        ],
        sample_config_rows(),
    )
}

pub(super) fn sample_seed() -> FixtureSeed {
    fixture_with(sample_finops())
}

pub(super) fn deterministic_rows(count: usize) -> Vec<FinOpsUsageRow> {
    (0..count)
        .map(|index| {
            let date = DATES
                .get(index.checked_rem(3).unwrap_or(0))
                .expect("a date cycles");
            let timestamp = format!("2025-01-01T00:00:00.{index:03}");
            FinOpsUsageRow {
                id: format!("row-{index:04}"),
                user_id: Some("alice".to_owned()),
                date: (*date).to_owned(),
                api_key: UsageApiKey::Key(secret("sk-alice-token")),
                model: Some("openai/gpt-5".to_owned()),
                model_group: Some("default".to_owned()),
                custom_llm_provider: Some("openai".to_owned()),
                mcp_namespaced_tool_name: None,
                endpoint: Some(format!("/v1/chat/{index}")),
                prompt_tokens: 1,
                completion_tokens: 1,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
                compression_saved_tokens: 0,
                compression_savings_spend: Decimal::ZERO,
                prompt_caching_savings_spend: Decimal::ZERO,
                gateway_injected_savings: Decimal::ZERO,
                autorouter_savings_spend: Decimal::ZERO,
                spend: Decimal::ZERO,
                api_requests: 1,
                successful_requests: 1,
                failed_requests: 0,
                total_response_time_ms: 10,
                timed_requests: 1,
                created_at: timestamp.clone(),
                updated_at: timestamp,
            }
        })
        .collect()
}

pub(super) fn expected_page(count: usize, limit: usize) -> Vec<String> {
    let mut indexes = (0..count).collect::<Vec<_>>();
    indexes.sort_by_key(|index| (Reverse(index.checked_rem(3).unwrap_or(0)), Reverse(*index)));
    indexes.truncate(limit);
    indexes
        .into_iter()
        .map(|index| format!("row-{index:04}"))
        .collect()
}

pub(super) fn seed_with_daily_usage(rows: Vec<FinOpsUsageRow>) -> FixtureSeed {
    fixture_with(with_lists(Vec::new(), Vec::new(), rows, Vec::new()))
}

pub(super) fn seed_with_organizations(organizations: Vec<OrganizationRow>) -> FixtureSeed {
    fixture_with(with_lists(
        organizations,
        Vec::new(),
        Vec::new(),
        Vec::new(),
    ))
}

pub(super) fn seed_with_key_metadata(key_metadata: Vec<KeyMetadataRow>) -> FixtureSeed {
    let mut finops = sample_finops();
    finops.key_metadata = key_metadata;
    fixture_with(finops)
}

pub(super) fn seed_with_config_rows(config_rows: Vec<ConfigRow>) -> FixtureSeed {
    fixture_with(with_lists(Vec::new(), Vec::new(), Vec::new(), config_rows))
}
