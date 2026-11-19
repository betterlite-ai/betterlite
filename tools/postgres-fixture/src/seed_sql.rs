use crate::{
    daily_usage, dashboard, dashboard_governance, discovery_skills, error::FixtureError, finops,
    scim, search_tools, seed::FixtureSeed, sql::BASELINE,
};
use secrecy::ExposeSecret as _;
use tokio_postgres::{Client, types::ToSql};

macro_rules! insert_rows {
    ($client:expr, $rows:expr, $sql:expr, $($field:ident),+) => {
        for row in $rows {
            let params: &[&(dyn ToSql + Sync)] = &[$(&row.$field),+];
            $client
                .execute($sql, params)
                .await
                .map_err(FixtureError::Database)?;
        }
    };
}

pub(crate) async fn seed_database(client: &Client, seed: &FixtureSeed) -> Result<(), FixtureError> {
    insert_rows!(
        client,
        &seed.users,
        "INSERT INTO \"LiteLLM_UserTable\" (\"user_id\",\"user_email\") VALUES ($1,$2)",
        user_id,
        user_email
    );
    insert_rows!(
        client,
        &seed.teams,
        "INSERT INTO \"LiteLLM_TeamTable\" (\"team_id\",\"team_alias\") VALUES ($1,$2)",
        team_id,
        team_alias
    );
    insert_rows!(
        client,
        &seed.memberships,
        "INSERT INTO \"LiteLLM_TeamMembership\" (\"user_id\",\"team_id\") VALUES ($1,$2)",
        user_id,
        team_id
    );
    insert_rows!(
        client,
        &seed.budgets,
        "INSERT INTO \"LiteLLM_BudgetTable\" (\"budget_id\",\"max_budget\",\"created_by\",\"updated_by\") VALUES ($1,$2::numeric::float8,$3,$4)",
        budget_id,
        max_budget,
        user_id,
        user_id
    );
    seed_keys(client, seed).await?;
    seed_extensions(client, seed).await
}

pub(crate) async fn schema_statements(seed: &FixtureSeed) -> Result<Vec<String>, FixtureError> {
    if seed.dashboard.is_some() {
        return Ok(dashboard::schema::verified_sql().await?);
    }
    if seed.daily_usage.is_some() || seed.finops.is_some() {
        return Ok(vec![format!("{BASELINE}{}", daily_usage::SCHEMA)]);
    }
    Ok(vec![BASELINE.to_owned()])
}

async fn seed_extensions(client: &Client, seed: &FixtureSeed) -> Result<(), FixtureError> {
    if let Some(rows) = &seed.daily_usage {
        daily_usage::seed(client, rows, &seed.keys).await?;
    }
    if let Some(finops_seed) = &seed.finops {
        finops::seed(client, finops_seed).await?;
    }
    if let Some(scim_seed) = &seed.scim {
        scim::seed(client, scim_seed).await?;
    }
    if let Some(discovery) = &seed.discovery_skills {
        discovery_skills::seed(client, discovery).await?;
    }
    if let Some(search) = &seed.search_tools {
        search_tools::seed(client, search).await?;
    }
    if let Some(dashboard_seed) = &seed.dashboard {
        dashboard::seed(client, dashboard_seed, seed).await?;
    }
    if let Some(governance) = &seed.dashboard_governance {
        dashboard_governance::seed(client, governance, seed).await?;
    }
    Ok(())
}

async fn seed_keys(client: &Client, seed: &FixtureSeed) -> Result<(), FixtureError> {
    for row in &seed.keys {
        let hash = FixtureSeed::key_hash(row.token.expose_secret());
        let params: &[&(dyn ToSql + Sync)] = &[&hash, &row.user_id, &row.team_id];
        client
            .execute(
                "INSERT INTO \"LiteLLM_VerificationToken\" (\"token\",\"user_id\",\"team_id\") VALUES ($1,$2,$3)",
                params,
            )
            .await
            .map_err(FixtureError::Database)?;
        if let Some(role) = row.role {
            let role_params: &[&(dyn ToSql + Sync)] = &[&role.as_wire(), &row.user_id];
            client
                .execute(
                    "UPDATE \"LiteLLM_UserTable\" SET \"user_role\" = $1 WHERE \"user_id\" = $2",
                    role_params,
                )
                .await
                .map_err(FixtureError::Database)?;
        }
    }
    Ok(())
}
