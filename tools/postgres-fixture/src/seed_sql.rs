use crate::{error::FixtureError, seed::FixtureSeed, sql::BASELINE};
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
    baseline(client).await?;
    seed_rows(client, seed).await?;
    Ok(())
}

async fn baseline(client: &Client) -> Result<(), FixtureError> {
    client
        .batch_execute(BASELINE)
        .await
        .map_err(FixtureError::Database)?;
    Ok(())
}

async fn seed_rows(client: &Client, seed: &FixtureSeed) -> Result<(), FixtureError> {
    users(client, seed).await?;
    teams(client, seed).await?;
    memberships(client, seed).await?;
    budgets(client, seed).await?;
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

async fn users(client: &Client, seed: &FixtureSeed) -> Result<(), FixtureError> {
    let rows = &seed.users;
    insert_rows!(
        client,
        rows,
        "INSERT INTO \"LiteLLM_UserTable\" (\"user_id\",\"user_email\") VALUES ($1,$2)",
        user_id,
        user_email
    );
    Ok(())
}

async fn teams(client: &Client, seed: &FixtureSeed) -> Result<(), FixtureError> {
    let rows = &seed.teams;
    insert_rows!(
        client,
        rows,
        "INSERT INTO \"LiteLLM_TeamTable\" (\"team_id\",\"team_alias\") VALUES ($1,$2)",
        team_id,
        team_alias
    );
    Ok(())
}

async fn memberships(client: &Client, seed: &FixtureSeed) -> Result<(), FixtureError> {
    let rows = &seed.memberships;
    insert_rows!(
        client,
        rows,
        "INSERT INTO \"LiteLLM_TeamMembership\" (\"user_id\",\"team_id\") VALUES ($1,$2)",
        user_id,
        team_id
    );
    Ok(())
}

async fn budgets(client: &Client, seed: &FixtureSeed) -> Result<(), FixtureError> {
    let rows = &seed.budgets;
    insert_rows!(
        client,
        rows,
        "INSERT INTO \"LiteLLM_BudgetTable\" (\"budget_id\",\"max_budget\",\"created_by\",\"updated_by\") VALUES ($1,$2,$3,$4)",
        budget_id,
        max_budget,
        user_id,
        user_id
    );
    Ok(())
}
