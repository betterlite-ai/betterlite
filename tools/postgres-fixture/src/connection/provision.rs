use secrecy::SecretString;
use tokio::task::JoinSet;

use super::cleanup::combine_errors;
use super::operations::{Work, create_resource, fill_resource};
use super::resource::Resource;
use super::template::template;
use super::{PostgresFixture, configured_options, quote_identifier};
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
    resource: Resource,
) -> Result<PostgresFixture, FixtureError> {
    seed.validate()?;
    let admin = configured_options(configured_url.as_ref())?.with_database("postgres")?;
    let quoted = quote_identifier(resource.name());
    match resource {
        Resource::Database(_) => {
            let statements = seed_sql::schema_statements(&seed).await?;
            let source = quote_identifier(&template(&admin, &statements).await?);
            let statement = format!("CREATE DATABASE {quoted} TEMPLATE {source}");
            let fixture = create_resource(&admin, resource, &statement).await?;
            fill_resource(fixture, Work::Seed(&seed)).await
        }
        Resource::Schema(_) => {
            let statement = format!("CREATE DATABASE {quoted}");
            let fixture = create_resource(&admin, resource, &statement).await?;
            let schema = [format!("CREATE SCHEMA {quoted}")];
            fill_resource(fixture, Work::Statements(&schema)).await
        }
    }
}
