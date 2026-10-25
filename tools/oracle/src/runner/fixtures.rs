use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_postgres_fixture::error::FixtureError;
use secrecy::{ExposeSecret, SecretString};

use crate::OracleError;
use crate::case::Case;

pub(super) struct FixtureGuard {
    pub(super) databases: Option<(PostgresFixture, PostgresFixture)>,
}

pub(super) fn provision(
    cases: &[Case],
    configured_url: Option<&SecretString>,
) -> Result<FixtureGuard, OracleError> {
    let first_case = cases.first().ok_or(OracleError::MissingCapture(0))?;
    let fixture_definition = first_case.database.clone();
    for current_case in cases {
        if current_case.database != fixture_definition {
            return Err(OracleError::DatabaseFixturesDiffer {
                cases: cases.iter().map(|item| item.name.clone()).collect(),
            });
        }
    }
    let Some(database) = fixture_definition else {
        return Ok(FixtureGuard { databases: None });
    };
    let exposed_url = configured_url.map(ExposeSecret::expose_secret);
    let first = PostgresFixture::provision_from_url(exposed_url, &database)
        .map_err(OracleError::DatabaseFixture)?;
    let second = match PostgresFixture::provision_from_url(exposed_url, &database) {
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
