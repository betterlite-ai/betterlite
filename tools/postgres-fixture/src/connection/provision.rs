use secrecy::SecretString;
use tokio::task::JoinSet;

use super::cleanup::{combine_errors, timed};
use super::operations::{create_database, run_fixture_operation, source_connection};
use super::{PostgresFixture, select_source};
use crate::error::FixtureError;
use crate::seed::FixtureSeed;
use crate::seed_sql;

#[must_use = "resolve the fixture or close the task to clean up unclaimed resources"]
pub struct ProvisionTask {
    pub(super) tasks: JoinSet<Result<PostgresFixture, FixtureError>>,
}

impl ProvisionTask {
    pub async fn resolve(&mut self) -> Result<PostgresFixture, FixtureError> {
        let result = self
            .tasks
            .join_next()
            .await
            .ok_or(FixtureError::TaskCompleted)?;
        result.map_err(FixtureError::DriverJoin)?
    }

    pub async fn close(mut self) -> Result<(), FixtureError> {
        let mut cleanup = Ok(());
        while let Some(result) = self.tasks.join_next().await {
            let outcome = match result {
                Ok(Ok(fixture)) => fixture.close_async().await,
                Ok(Err(error)) => Err(error),
                Err(source) => Err(FixtureError::DriverJoin(source)),
            };
            cleanup = combine_errors(cleanup, outcome);
        }
        cleanup
    }
}

pub(super) async fn provision_inner(
    configured_url: Option<SecretString>,
    seed: FixtureSeed,
    database: String,
) -> Result<PostgresFixture, FixtureError> {
    seed.validate()?;
    let source = select_source(configured_url.as_ref())?;
    let (admin, container) = source_connection(source).await?;
    let admin_url = admin.with_database("postgres");
    create_database(&admin_url, &database).await?;
    let options = admin.with_database(&database);
    run_fixture_operation(&options, &admin, &database, move |client| {
        Box::pin(
            async move { timed("seed database", seed_sql::seed_database(client, &seed)).await? },
        )
    })
    .await?;
    Ok(PostgresFixture {
        options,
        database,
        container,
        drivers: JoinSet::new(),
    })
}
