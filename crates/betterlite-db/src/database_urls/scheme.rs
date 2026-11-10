use crate::database_url_policy::policy::{unsupported_db_scheme, unsupported_db_scheme_message};
use crate::database_urls::environment::EnvironmentSource;
use crate::database_urls::error::DatabaseUrlError;
use crate::database_urls::settings::DatabaseUrlSettings;

pub fn validate_component_schemes(settings: &DatabaseUrlSettings) -> Result<(), DatabaseUrlError> {
    for (env_var, candidate) in [
        ("DATABASE_URL", settings.database_url.as_deref()),
        ("DIRECT_URL", settings.direct_url.as_deref()),
        (
            "DATABASE_URL_READ_REPLICA",
            settings.database_url_read_replica.as_deref(),
        ),
    ] {
        let Some(url) = candidate.filter(|value| !value.is_empty()) else {
            continue;
        };
        let Some(scheme) = unsupported_db_scheme(url)? else {
            continue;
        };
        return Err(DatabaseUrlError::UnsupportedScheme {
            message: unsupported_db_scheme_message(env_var, &scheme),
        });
    }
    Ok(())
}

// The CLI checks the variables it is about to hand to the migration runner, where an empty value is present and rejected
pub fn validate_cli_urls<S>(environment: &S) -> Result<(), DatabaseUrlError>
where
    S: EnvironmentSource,
{
    for env_var in ["DATABASE_URL", "DIRECT_URL"] {
        let Some(url) = environment.lookup(env_var) else {
            continue;
        };
        let Some(scheme) = unsupported_db_scheme(&url)? else {
            continue;
        };
        return Err(DatabaseUrlError::UnsupportedScheme {
            message: unsupported_db_scheme_message(env_var, &scheme),
        });
    }
    Ok(())
}
