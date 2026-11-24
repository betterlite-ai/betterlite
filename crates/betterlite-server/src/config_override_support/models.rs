use betterlite_config::config_override::{
    CYBERARK_DESCRIPTION, CYBERARK_FIELDS, HASHICORP_VAULT_DESCRIPTION, HASHICORP_VAULT_FIELDS,
    OverrideField,
};
use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object as JsonMap;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize, strum::Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum OverrideKind {
    HashicorpVault,
    CyberArk,
}

impl OverrideKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HashicorpVault => "hashicorp_vault",
            Self::CyberArk => "cyberark",
        }
    }

    #[must_use]
    pub const fn fields(self) -> &'static [OverrideField] {
        match self {
            Self::HashicorpVault => &HASHICORP_VAULT_FIELDS,
            Self::CyberArk => &CYBERARK_FIELDS,
        }
    }

    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::HashicorpVault => HASHICORP_VAULT_DESCRIPTION,
            Self::CyberArk => CYBERARK_DESCRIPTION,
        }
    }
}

// With no Debug derive, the plain string fields a request carries never reach log or panic text.
#[derive(Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct HashicorpVaultConfig {
    pub vault_addr: Option<String>,
    pub vault_token: Option<String>,
    pub approle_role_id: Option<String>,
    pub approle_secret_id: Option<String>,
    pub approle_mount_path: Option<String>,
    pub client_cert: Option<String>,
    pub client_key: Option<String>,
    pub vault_cert_role: Option<String>,
    pub vault_namespace: Option<String>,
    pub vault_login_namespace: Option<String>,
    pub vault_secret_namespace: Option<String>,
    pub vault_mount_name: Option<String>,
    pub vault_path_prefix: Option<String>,
}

#[derive(Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct CyberArkConfig {
    pub cyberark_api_base: Option<String>,
    pub cyberark_account: Option<String>,
    pub cyberark_username: Option<String>,
    pub cyberark_api_key: Option<String>,
    pub client_cert: Option<String>,
    pub client_key: Option<String>,
    pub ssl_verify: Option<String>,
    pub refresh_interval: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ConfigOverrideSettingsResponse {
    pub config_type: String,
    pub values: JsonMap,
    pub field_schema: JsonValue,
}

// The UI schema LiteLLM derives from the pydantic model: its docstring and each field's description and non-null type.
#[must_use]
pub fn field_schema(kind: OverrideKind) -> JsonValue {
    let properties = kind
        .fields()
        .iter()
        .map(|field| {
            let entry = [
                (
                    "description".to_owned(),
                    JsonValue::String(field.description.to_owned()),
                ),
                ("type".to_owned(), JsonValue::String("string".to_owned())),
            ]
            .into_iter()
            .collect::<JsonMap>();
            (field.name.to_owned(), JsonValue::Object(entry))
        })
        .collect::<JsonMap>();
    let schema = [
        (
            "description".to_owned(),
            JsonValue::String(kind.description().to_owned()),
        ),
        ("properties".to_owned(), JsonValue::Object(properties)),
    ]
    .into_iter()
    .collect::<JsonMap>();
    JsonValue::Object(schema)
}
