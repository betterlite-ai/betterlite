use secrecy::SecretString;
use tokio::task::JoinSet;

use super::cleanup::combine_errors;
use super::operations::{create_resource, seed_fixture, source_connection};
use super::resource::{Ownership, Resource};
use super::{PostgresFixture, select_source};
use crate::error::FixtureError;
use crate::seed::FixtureSeed;

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
    let source = select_source(configured_url.as_ref())?;
    let (admin, container) = source_connection(source).await?;
    let admin_url = match &resource {
        Resource::Database(_) => admin.with_database("postgres"),
        Resource::Schema(_) => admin,
    };
    let options = resource.connection(&admin_url)?;
    create_resource(&admin_url, &resource).await?;
    if matches!(&resource, Resource::Database(_)) {
        seed_fixture(&options, &admin_url, &resource, &seed).await?;
    }
    Ok(PostgresFixture {
        options,
        ownership: Ownership {
            resource,
            admin: Box::new(admin_url),
        },
        container,
    })
}
