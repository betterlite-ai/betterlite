use secrecy::ExposeSecret as _;
use tokio_postgres::Client;
use tokio_postgres::types::ToSql;

use super::FinOpsFixtureError;
use super::model::{ConfigRow, FinOpsUsageRow, KeyMetadataRow, OrganizationRow, UsageApiKey};
use crate::seed::FixtureSeed;

const ORGANIZATION_INSERT: &str = "INSERT INTO \"LiteLLM_OrganizationTable\" (\"organization_id\",\"organization_alias\",\"budget_id\",\"models\",\"created_by\",\"updated_by\") VALUES ($1,$2,$3,$4,$5,$6)";

const KEY_METADATA_UPDATE: &str = "UPDATE \"LiteLLM_VerificationToken\" SET \"key_alias\" = COALESCE($1, \"key_alias\"), \"organization_id\" = COALESCE($2, \"organization_id\") WHERE \"token\" = $3";

// The key's team comes from the seeded token row, so the fixture never rewrites membership.
const TEAM_METADATA_UPDATE: &str = "UPDATE \"LiteLLM_TeamTable\" SET \"organization_id\" = $1 WHERE \"team_id\" = (SELECT \"team_id\" FROM \"LiteLLM_VerificationToken\" WHERE \"token\" = $2)";

const DAILY_USAGE_INSERT: &str = "INSERT INTO \"LiteLLM_DailyUserSpend\" (\"id\",\"user_id\",\"date\",\"api_key\",\"model\",\"model_group\",\"custom_llm_provider\",\"mcp_namespaced_tool_name\",\"endpoint\",\"prompt_tokens\",\"completion_tokens\",\"cache_read_input_tokens\",\"cache_creation_input_tokens\",\"compression_saved_tokens\",\"compression_savings_spend\",\"prompt_caching_savings_spend\",\"gateway_injected_caching_savings_spend\",\"autorouter_savings_spend\",\"spend\",\"api_requests\",\"successful_requests\",\"failed_requests\",\"total_response_time_ms\",\"timed_requests\",\"created_at\",\"updated_at\") VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15::numeric::double precision,$16::numeric::double precision,$17::numeric::double precision,$18::numeric::double precision,$19::numeric::double precision,$20,$21,$22,$23,$24,$25::text::timestamp(3),$26::text::timestamp(3))";

const CONFIG_INSERT: &str = "INSERT INTO \"LiteLLM_Config\" (\"param_name\",\"param_value\") VALUES ($1, to_jsonb($2::text))";

async fn execute(
    client: &Client,
    statement: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Result<(), FinOpsFixtureError> {
    client
        .execute(statement, params)
        .await
        .map_err(FinOpsFixtureError::Database)?;
    Ok(())
}

pub(super) async fn organizations(
    client: &Client,
    rows: &[OrganizationRow],
) -> Result<(), FinOpsFixtureError> {
    for row in rows {
        execute(
            client,
            ORGANIZATION_INSERT,
            &[
                &row.organization_id,
                &row.organization_alias,
                &row.budget_id,
                &row.models,
                &row.created_by,
                &row.updated_by,
            ],
        )
        .await?;
    }
    Ok(())
}

pub(super) async fn key_metadata(
    client: &Client,
    rows: &[KeyMetadataRow],
) -> Result<(), FinOpsFixtureError> {
    for row in rows {
        let token = FixtureSeed::key_hash(row.token.expose_secret());
        execute(
            client,
            KEY_METADATA_UPDATE,
            &[&row.key_alias, &row.organization_id, &token],
        )
        .await?;
        if let Some(organization_id) = &row.team_organization_id {
            execute(client, TEAM_METADATA_UPDATE, &[&organization_id, &token]).await?;
        }
    }
    Ok(())
}

struct MoneyText {
    compression: String,
    prompt_caching: String,
    gateway_injected: String,
    autorouter: String,
    spend: String,
}

fn money_text(row: &FinOpsUsageRow) -> MoneyText {
    MoneyText {
        compression: row.compression_savings_spend.to_string(),
        prompt_caching: row.prompt_caching_savings_spend.to_string(),
        gateway_injected: row.gateway_injected_savings.to_string(),
        autorouter: row.autorouter_savings_spend.to_string(),
        spend: row.spend.to_string(),
    }
}

pub(super) async fn daily_usage(
    client: &Client,
    rows: &[FinOpsUsageRow],
) -> Result<(), FinOpsFixtureError> {
    for row in rows {
        let api_key = match &row.api_key {
            UsageApiKey::Key(token) => FixtureSeed::key_hash(token.expose_secret()),
            UsageApiKey::OrphanHash(hash) => hash.clone(),
        };
        execute_usage(client, row, &api_key, &money_text(row)).await?;
    }
    Ok(())
}

async fn execute_usage(
    client: &Client,
    row: &FinOpsUsageRow,
    api_key: &str,
    money: &MoneyText,
) -> Result<(), FinOpsFixtureError> {
    execute(
        client,
        DAILY_USAGE_INSERT,
        &[
            &row.id,
            &row.user_id,
            &row.date,
            &api_key,
            &row.model,
            &row.model_group,
            &row.custom_llm_provider,
            &row.mcp_namespaced_tool_name,
            &row.endpoint,
            &row.prompt_tokens,
            &row.completion_tokens,
            &row.cache_read_input_tokens,
            &row.cache_creation_input_tokens,
            &row.compression_saved_tokens,
            &money.compression,
            &money.prompt_caching,
            &money.gateway_injected,
            &money.autorouter,
            &money.spend,
            &row.api_requests,
            &row.successful_requests,
            &row.failed_requests,
            &row.total_response_time_ms,
            &row.timed_requests,
            &row.created_at,
            &row.updated_at,
        ],
    )
    .await
}

pub(super) async fn config_rows(
    client: &Client,
    rows: &[ConfigRow],
) -> Result<(), FinOpsFixtureError> {
    for row in rows {
        let value = row
            .serialized_value
            .as_ref()
            .map(|secret| secret.expose_secret().to_owned());
        execute(client, CONFIG_INSERT, &[&row.name.as_wire(), &value]).await?;
    }
    Ok(())
}
