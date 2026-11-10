use crate::database_url_policy::query::add_missing_query_params;
use crate::database_urls::endpoint::quote_plus;
use crate::database_urls::environment::EnvironmentSource;
use crate::database_urls::error::DatabaseUrlError;
use crate::database_urls::settings::from_env;
use crate::database_urls::tls::tls_params;

// The older CLI assembler reads only DATABASE_USERNAME and never a port; it loads settings only once its required fields pass
pub fn construct_cli_database_url<S>(environment: &S) -> Result<Option<String>, DatabaseUrlError>
where
    S: EnvironmentSource,
{
    let host_value = required(environment, "DATABASE_HOST");
    let username_value = required(environment, "DATABASE_USERNAME");
    let name_value = required(environment, "DATABASE_NAME");
    let (Some(host), Some(username), Some(name)) = (host_value, username_value, name_value) else {
        return Ok(None);
    };
    let password_value = required(environment, "DATABASE_PASSWORD");
    let schema_value = required(environment, "DATABASE_SCHEMA");
    let mut url = cli_url(&username, password_value.as_deref(), &host, &name);
    if let Some(schema) = schema_value {
        url.push_str("?schema=");
        url.push_str(&schema);
    }
    let settings = from_env(environment)?;
    Ok(Some(add_missing_query_params(
        &url,
        &tls_params(&settings),
    )?))
}

fn required<S>(environment: &S, name: &str) -> Option<String>
where
    S: EnvironmentSource,
{
    if let Some(value) = environment.lookup(name).filter(|value| !value.is_empty()) {
        return Some(value);
    }
    None
}

fn cli_url(username: &str, password: Option<&str>, host: &str, name: &str) -> String {
    let user = quote_plus(username);
    let database = quote_plus(name);
    password.map_or_else(
        || format!("postgresql://{user}@{host}/{database}"),
        |secret| {
            format!(
                "postgresql://{user}:{}@{host}/{database}",
                quote_plus(secret)
            )
        },
    )
}
