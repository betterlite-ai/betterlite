use crate::database_urls::environment::EnvironmentSource;

pub const POOLED_MARKER_ENV_VAR: &str = "LITELLM_PGBOUNCER_POOLED_DATABASE_URL";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenToggles {
    pub iam_token_db_auth: bool,
    pub azure_postgresql_auth: bool,
}

// pgbouncer.py compares the marker to exactly "true": "True", "1" and an empty value all leave token auth on
#[must_use]
pub fn database_url_is_pooled<S>(environment: &S) -> bool
where
    S: EnvironmentSource,
{
    environment
        .lookup(POOLED_MARKER_ENV_VAR)
        .is_some_and(|value| value == "true")
}
