use std::collections::BTreeMap;

use secrecy::SecretString;

use crate::database_url_policy::query::add_missing_query_params;
use crate::database_urls::endpoint::quote_plus;
use crate::database_urls::error::DatabaseUrlError;
use crate::database_urls::settings::from_env;
use crate::database_urls::tls::tls_params;

// The older CLI assembler reads only DATABASE_USERNAME and never a port; it loads settings only once its required fields pass
pub fn construct_cli_database_url(
    environment: &BTreeMap<String, String>,
) -> Result<Option<SecretString>, DatabaseUrlError> {
    let host_value = environment
        .get("DATABASE_HOST")
        .filter(|value| !value.is_empty());
    let username_value = environment
        .get("DATABASE_USERNAME")
        .filter(|value| !value.is_empty());
    let name_value = environment
        .get("DATABASE_NAME")
        .filter(|value| !value.is_empty());
    let (Some(host), Some(username), Some(name)) = (host_value, username_value, name_value) else {
        return Ok(None);
    };
    let password_value = environment
        .get("DATABASE_PASSWORD")
        .filter(|value| !value.is_empty());
    let schema_value = environment
        .get("DATABASE_SCHEMA")
        .filter(|value| !value.is_empty());
    let mut url = cli_url(username, password_value.map(String::as_str), host, name);
    if let Some(schema) = schema_value {
        url.push_str("?schema=");
        url.push_str(schema);
    }
    let settings = from_env(environment)?;
    let updated = add_missing_query_params(&url, &tls_params(&settings))?;
    Ok(Some(SecretString::from(updated)))
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
