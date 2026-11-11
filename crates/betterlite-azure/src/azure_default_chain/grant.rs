use secrecy::SecretString;

#[derive(Debug, Clone)]
pub enum LinkError {
    Unavailable(String),
    Authentication(String),
}

#[derive(Debug, Clone)]
pub struct ChainToken {
    pub token: SecretString,
    pub expires_on: Option<i64>,
    pub token_type: Option<String>,
    pub refresh_on: Option<i64>,
}

pub trait Grant {
    fn token(&self) -> Result<ChainToken, LinkError>;
}

#[derive(Debug, Clone)]
pub struct ClientSecretRecord {
    pub tenant_id: String,
    pub client_id: String,
    pub client_secret: SecretString,
    pub scope: String,
}

#[derive(Debug, Clone)]
pub struct CertificateRecord {
    pub tenant_id: String,
    pub client_id: String,
    pub certificate_path: String,
    pub password: Option<SecretString>,
    pub send_certificate_chain: bool,
    pub scope: String,
}

#[derive(Debug, Clone)]
pub struct UsernamePasswordRecord {
    pub client_id: String,
    pub username: String,
    pub password: SecretString,
    pub tenant_id: Option<String>,
    pub scope: String,
}

#[derive(Debug, Clone)]
pub struct WorkloadRecord {
    pub client_id: String,
    pub tenant_id: String,
    pub token_file: String,
    pub scope: String,
}

// the two SDK chain flags move together: DAC sets both, a plain chain only the first
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainPlacement {
    Standalone,
    Chained,
    DefaultCredential,
}

impl ChainPlacement {
    #[must_use]
    pub const fn within_credential_chain(self) -> bool {
        match self {
            Self::Standalone => false,
            Self::Chained | Self::DefaultCredential => true,
        }
    }

    #[must_use]
    pub const fn within_default_credential(self) -> bool {
        match self {
            Self::DefaultCredential => true,
            Self::Standalone | Self::Chained => false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ManagedIdentityRecord {
    pub client_id: Option<String>,
    pub exclude_workload_identity: bool,
    pub imds_probe: Option<bool>,
    pub placement: ChainPlacement,
    pub scope: String,
}

// the factory is the seam a fixture drives; production supplies native grants for every method
pub trait Grants {
    type Grant<'grants>: Grant
    where
        Self: 'grants;

    fn client_secret<'grants>(
        &'grants self,
        record: &ClientSecretRecord,
    ) -> Result<Self::Grant<'grants>, GrantError>;
    fn certificate<'grants>(
        &'grants self,
        record: &CertificateRecord,
    ) -> Result<Self::Grant<'grants>, GrantError>;
    fn username_password<'grants>(
        &'grants self,
        record: &UsernamePasswordRecord,
    ) -> Result<Self::Grant<'grants>, GrantError>;
    fn workload<'grants>(
        &'grants self,
        record: &WorkloadRecord,
    ) -> Result<Self::Grant<'grants>, GrantError>;
    fn managed_identity<'grants>(
        &'grants self,
        record: &ManagedIdentityRecord,
    ) -> Result<Self::Grant<'grants>, GrantError>;
}

#[derive(Debug, Clone)]
pub struct GrantError {
    pub credential: &'static str,
    pub message: String,
}
