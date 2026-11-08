use super::types::Environment;

mod tests;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EnvironmentError {
    #[error("invalid LITELM_ENVIRONMENT {environment:?}; use production, development or staging")]
    Invalid { environment: String },
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
