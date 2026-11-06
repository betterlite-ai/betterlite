use chrono::{DateTime, SecondsFormat, TimeZone, Utc};

use super::types::Environment;

mod tests;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EnvironmentError {
    #[error("invalid LITELM_ENVIRONMENT {environment:?}; use production, development or staging")]
    Invalid { environment: String },
}

#[must_use]
pub fn to_zod_compatible_iso<Tz>(moment: &DateTime<Tz>) -> String
where
    Tz: TimeZone,
{
    moment
        .with_timezone(&Utc)
        // DeepEval's zod parser expects exactly three fraction digits, which chrono writes whether or not they are zero
        .to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub fn validate_environment(environment: &str) -> Result<Environment, EnvironmentError> {
    match environment {
        "production" => Ok(Environment::Production),
        "development" => Ok(Environment::Development),
        "staging" => Ok(Environment::Staging),
        _ => Err(EnvironmentError::Invalid {
            environment: environment.to_owned(),
        }),
    }
}
