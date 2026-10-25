use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_postgres_fixture::error::FixtureError;

use crate::OracleError;
use crate::case::Case;

pub(super) struct FixtureGuard {
    pub(super) first: Option<PostgresFixture>,
    pub(super) second: Option<PostgresFixture>,
}

pub(super) fn provision(
    cases: &[Case],
    configured_url: Option<&str>,
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
        return Ok(FixtureGuard {
            first: None,
            second: None,
        });
    };
    let first = PostgresFixture::provision_from_url(configured_url, &database)
        .map_err(OracleError::DatabaseFixture)?;
    let second = match PostgresFixture::provision_from_url(configured_url, &database) {
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
        first: Some(first),
        second: Some(second),
    })
}

impl FixtureGuard {
    pub(super) fn close(mut self) -> Result<(), FixtureError> {
        let first_result = self.first.take().map_or(Ok(()), PostgresFixture::close);
        let second_result = self.second.take().map_or(Ok(()), PostgresFixture::close);
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
