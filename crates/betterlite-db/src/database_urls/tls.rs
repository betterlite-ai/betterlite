use crate::database_url_policy::query::QueryValue;
use crate::database_urls::settings::DatabaseUrlSettings;

// DATABASE_SSLMODE wins when nonempty; a root certificate alone means verify-full, never libpq's unverified prefer
#[must_use]
pub fn tls_params(settings: &DatabaseUrlSettings) -> Vec<(String, QueryValue)> {
    let sslmode = settings
        .database_sslmode
        .clone()
        .filter(|mode| !mode.is_empty())
        .or_else(|| {
            settings
                .database_sslrootcert
                .as_deref()
                .filter(|root| !root.is_empty())
                .map(|_root| "verify-full".to_owned())
        });
    let mut params = Vec::new();
    if let Some(mode) = sslmode {
        params.push(("sslmode".to_owned(), QueryValue::Text(mode)));
    }
    if let Some(root) = settings
        .database_sslrootcert
        .clone()
        .filter(|root| !root.is_empty())
    {
        params.push(("sslrootcert".to_owned(), QueryValue::Text(root)));
    }
    params
}
