use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyManagementSystem {
    AwsKms,
    AwsSecretManager,
    GoogleSecretManager,
    GoogleKms,
    HashicorpVault,
    Cyberark,
    AzureKeyVault,
    Local,
    Custom,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretAccessMode {
    #[default]
    ReadOnly,
    WriteOnly,
    ReadAndWrite,
}

#[path = "secret_types/tests.rs"]
mod tests;
