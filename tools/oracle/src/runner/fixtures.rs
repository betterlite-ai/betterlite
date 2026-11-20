use std::path::Path;

use betterlite_oracle_runner::helpers::{
    generate_prisma_client, prisma::fill_key_models, sync_litellm,
};
use betterlite_postgres_fixture::connection::{
    DatabaseNameGenerator, PostgresFixture, RandomDatabaseName,
};
use betterlite_postgres_fixture::error::FixtureError;
use betterlite_postgres_fixture::seed::DatabaseFixture;
use secrecy::{ExposeSecret as _, SecretString};

use crate::OracleError;
use crate::case::Case;

pub(super) struct FixtureGuard {
    pub(super) databases: Option<(PostgresFixture, PostgresFixture)>,
}

// Every case in a group shares one fixture definition or none; a mismatch is a case-authoring bug.
fn shared_fixture(cases: &[Case]) -> Result<Option<&DatabaseFixture>, OracleError> {
    let first_case = cases.first().ok_or(OracleError::MissingCapture(0))?;
    let fixture_definition = &first_case.database;
    for current_case in cases {
        if current_case.database != *fixture_definition {
            return Err(OracleError::DatabaseFixturesDiffer {
                cases: cases.iter().map(|item| item.name.clone()).collect(),
            });
        }
    }
    Ok(fixture_definition.as_ref())
}

pub(super) fn provision(
    cases: &[Case],
    configured_url: Option<&SecretString>,
) -> Result<FixtureGuard, OracleError> {
    let Some(database) = shared_fixture(cases)? else {
        return Ok(FixtureGuard { databases: None });
    };
    provision_pair(configured_url, database, &mut RandomDatabaseName)
}

pub(super) fn provision_with_generator<G>(
    cases: &[Case],
    configured_url: Option<&SecretString>,
    generator: &mut G,
) -> Result<FixtureGuard, OracleError>
where
    G: DatabaseNameGenerator,
{
    let Some(database) = shared_fixture(cases)? else {
        return Ok(FixtureGuard { databases: None });
    };
    provision_pair(configured_url, database, generator)
}

fn provision_pair<G>(
    configured_url: Option<&SecretString>,
    database: &DatabaseFixture,
    generator: &mut G,
) -> Result<FixtureGuard, OracleError>
where
    G: DatabaseNameGenerator,
{
    // the fixture's gateways run from the synced venv, and LiteLLM's Prisma client must exist before it boots
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    sync_litellm(&root)?;
    generate_prisma_client(&root)?;
    let first = PostgresFixture::provision_with_generator_sync(configured_url, database, generator)
        .map_err(OracleError::DatabaseFixture)?;
    let second =
        match PostgresFixture::provision_with_generator_sync(configured_url, database, generator) {
            Ok(second_database) => second_database,
            Err(error) => {
                let cleanup_result = first.close();
                if let Err(cleanup_error) = cleanup_result {
                    return Err(OracleError::DatabaseCleanup(FixtureError::SetupCleanup {
                        setup: Box::new(error),
                        cleanup: Box::new(cleanup_error),
                    }));
                }
                return Err(OracleError::DatabaseFixture(error));
            }
        };
    fill_seeded_key_models(&root, database, first, second)
}

// LiteLLM reads a key through a view that rejects a NULL models column, and the fixture's key seed leaves it empty
fn fill_seeded_key_models(
    root: &Path,
    database: &DatabaseFixture,
    first: PostgresFixture,
    second: PostgresFixture,
) -> Result<FixtureGuard, OracleError> {
    let filled = if database.seed.keys.is_empty() {
        Ok(())
    } else {
        fill_key_models(root, first.options.url.expose_secret())
            .and_then(|()| fill_key_models(root, second.options.url.expose_secret()))
            .map_err(OracleError::HelperRun)
    };
    let guard = FixtureGuard {
        databases: Some((first, second)),
    };
    match filled {
        Ok(()) => Ok(guard),
        Err(operation) => match guard.close() {
            Ok(()) => Err(operation),
            Err(cleanup) => Err(OracleError::OperationCleanup {
                operation: Box::new(operation),
                cleanup,
            }),
        },
    }
}

impl FixtureGuard {
    pub(super) fn close(self) -> Result<(), FixtureError> {
        let Some((first, second)) = self.databases else {
            return Ok(());
        };
        let first_result = first.close();
        let second_result = second.close();
        match (first_result, second_result) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
            (Err(first_error), Err(second_error)) => Err(FixtureError::MultipleCleanup {
                first: Box::new(first_error),
                second: Box::new(second_error),
            }),
        }
    }
}
