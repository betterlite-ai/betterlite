use crate::{error::FixtureError, seed::FixtureSeed, sql::BASELINE};
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
    client
        .batch_execute(BASELINE)
        .await
        .map_err(FixtureError::Database)?;
    seed_rows(client, seed).await?;
    keys(client, seed).await?;
    Ok(())
}

async fn seed_rows(client: &Client, seed: &FixtureSeed) -> Result<(), FixtureError> {
    insert_rows!(
        client,
        &seed.users,
        "INSERT INTO \"LiteLLM_UserTable\" VALUES ($1,$2)",
        user_id,
        user_email
    );
    insert_rows!(
        client,
        &seed.teams,
        "INSERT INTO \"LiteLLM_TeamTable\" VALUES ($1,$2)",
        team_id,
        team_alias
    );
    insert_rows!(
        client,
        &seed.memberships,
        "INSERT INTO \"LiteLLM_TeamMembership\" VALUES ($1,$2)",
        user_id,
        team_id
    );
    insert_rows!(
        client,
        &seed.budgets,
        "INSERT INTO \"LiteLLM_BudgetTable\" VALUES ($1,$2,$3)",
        budget_id,
        user_id,
        max_budget
    );
    insert_rows!(
        client,
        &seed.jwt_mappings,
        "INSERT INTO \"LiteLLM_JWTMapping\" VALUES ($1,$2)",
        subject,
        user_id
    );
    Ok(())
}

async fn keys(client: &Client, seed: &FixtureSeed) -> Result<(), FixtureError> {
    for row in &seed.keys {
        let hash = FixtureSeed::key_hash(&row.token);
        let params: &[&(dyn ToSql + Sync)] = &[&hash, &row.user_id, &row.team_id, &row.role];
        client
            .execute(
                "INSERT INTO \"LiteLLM_VerificationToken\" VALUES ($1,$2,$3,$4)",
                params,
            )
            .await
            .map_err(FixtureError::Database)?;
    }
    Ok(())
}
