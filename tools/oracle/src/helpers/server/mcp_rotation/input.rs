use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ServerInput {
    pub(crate) server_id: String,
    #[serde(default)]
    pub(crate) credentials: Option<Value>,
    #[serde(default)]
    pub(crate) env_vars: Option<Value>,
    #[serde(default)]
    pub(crate) static_headers: Option<Value>,
    #[serde(default)]
    pub(crate) env: Option<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OAuthInput {
    pub(crate) server_id: String,
    #[serde(default)]
    pub(crate) credentials: Option<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CredentialInput {
    pub(crate) user_id: String,
    pub(crate) server_id: String,
    pub(crate) credential_b64: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EnvVarInput {
    pub(crate) user_id: String,
    pub(crate) server_id: String,
    pub(crate) values_b64: String,
}

// Internally tagged enums hand each field a ContentDeserializer that cannot serve JsonValue's Box<RawValue>.
#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    RotateServers {
        salt_key: String,
        new_master_key: String,
        nonce_hex: String,
        touched_by: String,
        servers: Vec<ServerInput>,
        oauth_clients: Vec<OAuthInput>,
        #[serde(default)]
        fail_at_write: Option<usize>,
    },
    RotateUserCredentials {
        salt_key: String,
        new_master_key: String,
        nonce_hex: String,
        rows: Vec<CredentialInput>,
        #[serde(default)]
        fail_at_write: Option<usize>,
    },
    RotateUserEnvVars {
        salt_key: String,
        new_master_key: String,
        nonce_hex: String,
        rows: Vec<EnvVarInput>,
        #[serde(default)]
        fail_at_write: Option<usize>,
    },
}
