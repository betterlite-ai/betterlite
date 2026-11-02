use betterlite_postgres_fixture::connection::{
    DatabaseNameGenerator, PostgresFixture, RandomDatabaseName,
};
use betterlite_postgres_fixture::error::FixtureError;
use betterlite_postgres_fixture::seed::DatabaseFixture;
use secrecy::SecretString;

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
    Ok(FixtureGuard {
        databases: Some((first, second)),
    })
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
