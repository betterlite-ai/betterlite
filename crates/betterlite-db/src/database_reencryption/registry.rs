use betterlite_secrets::secret_codec::CALLBACK_VAR_ENCRYPTED_PREFIX;

pub const DISCOVERY_SQL: &str = "SELECT table_name, column_name FROM information_schema.columns \
WHERE table_schema = ANY (current_schemas(false))";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecretColumn {
    pub table: &'static str,
    pub primary_key: &'static str,
    pub column: &'static str,
    pub is_json: bool,
    pub marked_only: bool,
}

#[derive(Debug, Clone, Copy)]
enum ColumnKind {
    Json,
    MarkedJson,
    Scalar,
}

// LiteLLM's _SECRET_COLUMNS order; the three kinds are contiguous in it
const REGISTRY: [(&str, &str, &str, ColumnKind); 19] = [
    (
        "LiteLLM_ProxyModelTable",
        "model_id",
        "litellm_params",
        ColumnKind::Json,
    ),
    (
        "LiteLLM_CredentialsTable",
        "credential_id",
        "credential_values",
        ColumnKind::Json,
    ),
    (
        "LiteLLM_Config",
        "param_name",
        "param_value",
        ColumnKind::Json,
    ),
    ("LiteLLM_SSOConfig", "id", "sso_settings", ColumnKind::Json),
    (
        "LiteLLM_CacheConfig",
        "id",
        "cache_settings",
        ColumnKind::Json,
    ),
    (
        "LiteLLM_ConfigOverrides",
        "config_type",
        "config_value",
        ColumnKind::Json,
    ),
    (
        "LiteLLM_MCPServerTable",
        "server_id",
        "credentials",
        ColumnKind::Json,
    ),
    (
        "LiteLLM_MCPServerTable",
        "server_id",
        "static_headers",
        ColumnKind::Json,
    ),
    (
        "LiteLLM_MCPServerTable",
        "server_id",
        "env_vars",
        ColumnKind::Json,
    ),
    (
        "LiteLLM_MCPServerTable",
        "server_id",
        "env",
        ColumnKind::Json,
    ),
    (
        "LiteLLM_MCPServerOAuthClient",
        "server_id",
        "credentials",
        ColumnKind::Json,
    ),
    (
        "LiteLLM_MCPUserCredentials",
        "id",
        "credential_b64",
        ColumnKind::Scalar,
    ),
    (
        "LiteLLM_MCPUserEnvVars",
        "id",
        "values_b64",
        ColumnKind::Scalar,
    ),
    (
        "LiteLLM_SSOIdentityAssertion",
        "user_id",
        "assertion_b64",
        ColumnKind::Scalar,
    ),
    (
        "LiteLLM_TeamTable",
        "team_id",
        "metadata",
        ColumnKind::MarkedJson,
    ),
    (
        "LiteLLM_VerificationToken",
        "token",
        "metadata",
        ColumnKind::MarkedJson,
    ),
    (
        "LiteLLM_UserTable",
        "user_id",
        "metadata",
        ColumnKind::MarkedJson,
    ),
    (
        "LiteLLM_DeletedTeamTable",
        "id",
        "metadata",
        ColumnKind::MarkedJson,
    ),
    (
        "LiteLLM_DeletedVerificationToken",
        "id",
        "metadata",
        ColumnKind::MarkedJson,
    ),
];

pub fn registered_columns() -> impl Iterator<Item = SecretColumn> {
    REGISTRY
        .iter()
        .map(|&(table, primary_key, column, kind)| SecretColumn {
            table,
            primary_key,
            column,
            is_json: !matches!(kind, ColumnKind::Scalar),
            marked_only: matches!(kind, ColumnKind::MarkedJson),
        })
}

#[must_use]
pub fn rows_sql(column: &SecretColumn) -> String {
    let name = column.column;
    let selected = if column.is_json {
        format!("\"{name}\"::text")
    } else {
        format!("\"{name}\"")
    };
    let marked = if column.marked_only {
        format!(" AND \"{name}\"::text LIKE '%{CALLBACK_VAR_ENCRYPTED_PREFIX}%'")
    } else {
        String::new()
    };
    format!(
        "SELECT \"{}\", {selected} FROM \"{}\" WHERE \"{name}\" IS NOT NULL{marked}",
        column.primary_key, column.table
    )
}

#[must_use]
pub fn update_sql(column: &SecretColumn) -> String {
    let name = column.column;
    let cast = if column.is_json { "::jsonb" } else { "" };
    format!(
        "UPDATE \"{}\" SET \"{name}\" = $1{cast} WHERE \"{}\" = $2 AND \"{name}\" = $3{cast}",
        column.table, column.primary_key
    )
}
