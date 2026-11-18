use std::io;

use betterlite_auth::resolved_key::auth::AuthError;
use betterlite_server::model_visibility::testing::FixtureError;

#[derive(Debug, thiserror::Error)]
pub(crate) enum HealthError {
    #[error("the health cache is unreachable")]
    Unreachable,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum Failure {
    #[error(transparent)]
    Fixture(#[from] FixtureError),
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error("the health cache cannot answer: {0}")]
    Health(#[from] HealthError),
    #[error(transparent)]
    Runtime(#[from] io::Error),
}
