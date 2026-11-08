use std::collections::BTreeMap;
use std::future::ready;
use std::sync::{Arc, Mutex, PoisonError};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use secrecy::{ExposeSecret as _, SecretString};

use crate::aws_kms::AwsKmsError;
use crate::custom_interface::{
    CustomParams, CustomSecretError, CustomSecretFuture, CustomSecretManager, CustomValue,
};
use crate::google_kms::GoogleKmsError;
use crate::google_secret_manager::GoogleSecretManagerError;
use crate::secret_read_dispatch::SecretManagerClient;
use crate::secret_read_dispatch::error::SecretDispatchError;
use crate::secret_read_dispatch::protocols::{
    AwsKmsClient, AwsSecretsManagerClient, AwsSecretsManagerError, AzureKeyVaultClient,
    AzureKeyVaultError, AzureSecret, GoogleKmsClient, GoogleKmsDecryptRequest,
    GoogleSecretManagerClient, InfisicalClient, InfisicalError, InfisicalSecret, SyncSecretReader,
};

pub(super) fn environment(name: &str, value: &str) -> BTreeMap<String, String> {
    BTreeMap::from([(name.to_owned(), value.to_owned())])
}

pub(super) fn empty_environment() -> BTreeMap<String, String> {
    BTreeMap::new()
}

pub(super) fn base64_of(text: &str) -> String {
    STANDARD.encode(text)
}

pub(super) fn assert_secret(
    result: Result<Option<SecretString>, SecretDispatchError>,
    expected: &str,
) {
    let secret = result
        .expect("the dispatch should succeed")
        .expect("the dispatch should return a secret");
    assert_eq!(secret.expose_secret(), expected);
}

pub(super) fn assert_no_secret(result: &Result<Option<SecretString>, SecretDispatchError>) {
    assert!(matches!(result, Ok(None)));
}

#[derive(Clone, Copy)]
pub(super) enum ClientKind {
    Azure,
    GoogleKms,
    AwsKms,
    AwsSecretManager,
    GoogleSecretManager,
    Vault,
    CyberArk,
    Custom,
    Infisical,
}

pub(super) fn azure_client<Client>(fake: &Arc<Client>) -> SecretManagerClient
where
    Client: AzureKeyVaultClient + 'static,
{
    let handle: Arc<dyn AzureKeyVaultClient> = Arc::<Client>::clone(fake);
    SecretManagerClient::AzureKeyVault(handle)
}

pub(super) fn google_kms_client<Client>(fake: &Arc<Client>) -> SecretManagerClient
where
    Client: GoogleKmsClient + 'static,
{
    let handle: Arc<dyn GoogleKmsClient> = Arc::<Client>::clone(fake);
    SecretManagerClient::GoogleKms(handle)
}

pub(super) fn aws_kms_client<Client>(fake: &Arc<Client>) -> SecretManagerClient
where
    Client: AwsKmsClient + 'static,
{
    let handle: Arc<dyn AwsKmsClient> = Arc::<Client>::clone(fake);
    SecretManagerClient::AwsKms(handle)
}

pub(super) fn aws_secrets_client<Client>(fake: &Arc<Client>) -> SecretManagerClient
where
    Client: AwsSecretsManagerClient + 'static,
{
    let handle: Arc<dyn AwsSecretsManagerClient> = Arc::<Client>::clone(fake);
    SecretManagerClient::AwsSecretManager(handle)
}

pub(super) fn google_secrets_client<Client>(fake: &Arc<Client>) -> SecretManagerClient
where
    Client: GoogleSecretManagerClient + 'static,
{
    let handle: Arc<dyn GoogleSecretManagerClient> = Arc::<Client>::clone(fake);
    SecretManagerClient::GoogleSecretManager(handle)
}

pub(super) fn vault_client<Client>(fake: &Arc<Client>) -> SecretManagerClient
where
    Client: SyncSecretReader + 'static,
{
    let handle: Arc<dyn SyncSecretReader> = Arc::<Client>::clone(fake);
    SecretManagerClient::Vault(handle)
}

pub(super) fn cyberark_client<Client>(fake: &Arc<Client>) -> SecretManagerClient
where
    Client: SyncSecretReader + 'static,
{
    let handle: Arc<dyn SyncSecretReader> = Arc::<Client>::clone(fake);
    SecretManagerClient::CyberArk(handle)
}

pub(super) fn custom_client<Client>(fake: &Arc<Client>) -> SecretManagerClient
where
    Client: CustomSecretManager + 'static,
{
    let handle: Arc<dyn CustomSecretManager> = Arc::<Client>::clone(fake);
    SecretManagerClient::Custom(handle)
}

pub(super) fn infisical_client<Client>(fake: &Arc<Client>) -> SecretManagerClient
where
    Client: InfisicalClient + 'static,
{
    let handle: Arc<dyn InfisicalClient> = Arc::<Client>::clone(fake);
    SecretManagerClient::Infisical(handle)
}

pub(super) struct AzureFake {
    value: Option<SecretString>,
    fail: bool,
    names: Mutex<Vec<String>>,
}

impl AzureFake {
    pub(super) fn names(&self) -> Vec<String> {
        self.names
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, secret_name: &str) {
        self.names
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(secret_name.to_owned());
    }
}

pub(super) fn azure_fake(value: Option<&str>) -> Arc<AzureFake> {
    Arc::new(AzureFake {
        value: value.map(SecretString::from),
        fail: false,
        names: Mutex::new(Vec::new()),
    })
}

pub(super) fn azure_failing_fake() -> Arc<AzureFake> {
    Arc::new(AzureFake {
        value: None,
        fail: true,
        names: Mutex::new(Vec::new()),
    })
}

impl AzureKeyVaultClient for AzureFake {
    fn get_secret(&self, secret_name: &str) -> Result<AzureSecret, AzureKeyVaultError> {
        self.record(secret_name);
        if self.fail {
            return Err(AzureKeyVaultError);
        }
        Ok(AzureSecret {
            value: self.value.clone(),
        })
    }
}

pub(super) struct GoogleKmsFake {
    resource_name: String,
    plaintext: Option<Vec<u8>>,
    requests: Mutex<Vec<(String, Vec<u8>)>>,
}

impl GoogleKmsFake {
    pub(super) fn requests(&self) -> Vec<(String, Vec<u8>)> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

pub(super) fn google_kms_fake(resource_name: &str, plaintext: Option<&[u8]>) -> Arc<GoogleKmsFake> {
    Arc::new(GoogleKmsFake {
        resource_name: resource_name.to_owned(),
        plaintext: plaintext.map(<[u8]>::to_vec),
        requests: Mutex::new(Vec::new()),
    })
}

impl GoogleKmsClient for GoogleKmsFake {
    fn resource_name(&self) -> &str {
        &self.resource_name
    }

    fn decrypt(&self, request: GoogleKmsDecryptRequest<'_>) -> Result<Vec<u8>, GoogleKmsError> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((request.name.to_owned(), request.ciphertext.to_vec()));
        self.plaintext.clone().ok_or(GoogleKmsError::ClientState)
    }
}

pub(super) struct AwsKmsFake {
    plaintext: Option<Vec<u8>>,
    blobs: Mutex<Vec<Vec<u8>>>,
}

impl AwsKmsFake {
    pub(super) fn blobs(&self) -> Vec<Vec<u8>> {
        self.blobs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

pub(super) fn aws_kms_fake(plaintext: Option<&[u8]>) -> Arc<AwsKmsFake> {
    Arc::new(AwsKmsFake {
        plaintext: plaintext.map(<[u8]>::to_vec),
        blobs: Mutex::new(Vec::new()),
    })
}

impl AwsKmsClient for AwsKmsFake {
    fn decrypt(&self, ciphertext_blob: &[u8]) -> Result<Vec<u8>, AwsKmsError> {
        self.blobs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ciphertext_blob.to_vec());
        self.plaintext.clone().ok_or(AwsKmsError::ClientState)
    }
}

pub(super) struct AwsSecretsFake {
    value: Option<SecretString>,
    fail: bool,
    calls: Mutex<Vec<(String, Option<String>)>>,
}

impl AwsSecretsFake {
    pub(super) fn calls(&self) -> Vec<(String, Option<String>)> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, secret_name: &str, primary_secret_name: Option<&str>) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((
                secret_name.to_owned(),
                primary_secret_name.map(str::to_owned),
            ));
    }
}

pub(super) fn aws_secrets_fake(value: Option<&str>) -> Arc<AwsSecretsFake> {
    Arc::new(AwsSecretsFake {
        value: value.map(SecretString::from),
        fail: false,
        calls: Mutex::new(Vec::new()),
    })
}

pub(super) fn aws_secrets_failing_fake() -> Arc<AwsSecretsFake> {
    Arc::new(AwsSecretsFake {
        value: None,
        fail: true,
        calls: Mutex::new(Vec::new()),
    })
}

impl AwsSecretsManagerClient for AwsSecretsFake {
    fn sync_read_secret(
        &self,
        secret_name: &str,
        primary_secret_name: Option<&str>,
    ) -> Result<Option<SecretString>, AwsSecretsManagerError> {
        self.record(secret_name, primary_secret_name);
        if self.fail {
            return Err(AwsSecretsManagerError);
        }
        Ok(self.value.clone())
    }
}

pub(super) struct GoogleSecretsFake {
    value: Option<SecretString>,
    fail: bool,
    names: Mutex<Vec<String>>,
}

impl GoogleSecretsFake {
    pub(super) fn names(&self) -> Vec<String> {
        self.names
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

pub(super) fn google_secrets_fake(value: Option<&str>) -> Arc<GoogleSecretsFake> {
    Arc::new(GoogleSecretsFake {
        value: value.map(SecretString::from),
        fail: false,
        names: Mutex::new(Vec::new()),
    })
}

pub(super) fn google_secrets_failing_fake() -> Arc<GoogleSecretsFake> {
    Arc::new(GoogleSecretsFake {
        value: None,
        fail: true,
        names: Mutex::new(Vec::new()),
    })
}

impl GoogleSecretManagerClient for GoogleSecretsFake {
    fn get_secret_from_google_secret_manager(
        &self,
        secret_name: &str,
    ) -> Result<Option<SecretString>, GoogleSecretManagerError> {
        self.names
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(secret_name.to_owned());
        if self.fail {
            return Err(GoogleSecretManagerError::Status {
                secret_name: secret_name.to_owned(),
                status: 500,
            });
        }
        Ok(self.value.clone())
    }
}

pub(super) struct SyncReaderFake {
    value: Option<SecretString>,
    names: Mutex<Vec<String>>,
}

impl SyncReaderFake {
    pub(super) fn names(&self) -> Vec<String> {
        self.names
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

pub(super) fn sync_reader_fake(value: Option<&str>) -> Arc<SyncReaderFake> {
    Arc::new(SyncReaderFake {
        value: value.map(SecretString::from),
        names: Mutex::new(Vec::new()),
    })
}

impl SyncSecretReader for SyncReaderFake {
    fn sync_read_secret(&self, secret_name: &str) -> Option<SecretString> {
        self.names
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(secret_name.to_owned());
        self.value.clone()
    }
}

pub(super) struct CustomFake {
    value: Option<SecretString>,
    failing_method: Option<&'static str>,
    calls: Mutex<Vec<(String, Option<CustomParams>)>>,
}

impl CustomFake {
    pub(super) fn calls(&self) -> Vec<(String, Option<CustomParams>)> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, secret_name: &str, optional_params: Option<&CustomParams>) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((secret_name.to_owned(), optional_params.cloned()));
    }
}

pub(super) fn custom_fake(value: Option<&str>) -> Arc<CustomFake> {
    Arc::new(CustomFake {
        value: value.map(SecretString::from),
        failing_method: None,
        calls: Mutex::new(Vec::new()),
    })
}

pub(super) fn custom_failing_fake() -> Arc<CustomFake> {
    Arc::new(CustomFake {
        value: None,
        failing_method: Some("sync_read_secret"),
        calls: Mutex::new(Vec::new()),
    })
}

impl CustomSecretManager for CustomFake {
    fn secret_manager_name(&self) -> &'static str {
        "custom_secret_manager"
    }

    fn async_read_secret<'borrowed>(
        &'borrowed self,
        _secret_name: &'borrowed str,
        _optional_params: Option<&'borrowed CustomParams>,
    ) -> CustomSecretFuture<'borrowed, Option<SecretString>> {
        Box::pin(ready(Ok(None)))
    }

    fn sync_read_secret(
        &self,
        secret_name: &str,
        optional_params: Option<&CustomParams>,
    ) -> Result<Option<SecretString>, CustomSecretError> {
        self.record(secret_name, optional_params);
        self.failing_method.map_or_else(
            || Ok(self.value.clone()),
            |method| {
                Err(CustomSecretError::NotImplemented {
                    manager: self.secret_manager_name().to_owned(),
                    method,
                })
            },
        )
    }
}

pub(super) fn custom_optional_params() -> CustomParams {
    BTreeMap::from([
        (
            "access_mode".to_owned(),
            CustomValue::String("read_only".to_owned()),
        ),
        (
            "primary_secret_name".to_owned(),
            CustomValue::String("primary".to_owned()),
        ),
    ])
}

pub(super) struct InfisicalFake {
    value: Option<SecretString>,
    fail: bool,
    names: Mutex<Vec<String>>,
}

impl InfisicalFake {
    pub(super) fn names(&self) -> Vec<String> {
        self.names
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

pub(super) fn infisical_fake(value: Option<&str>) -> Arc<InfisicalFake> {
    Arc::new(InfisicalFake {
        value: value.map(SecretString::from),
        fail: false,
        names: Mutex::new(Vec::new()),
    })
}

pub(super) fn infisical_failing_fake() -> Arc<InfisicalFake> {
    Arc::new(InfisicalFake {
        value: None,
        fail: true,
        names: Mutex::new(Vec::new()),
    })
}

impl InfisicalClient for InfisicalFake {
    fn get_secret(&self, secret_name: &str) -> Result<InfisicalSecret, InfisicalError> {
        self.names
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(secret_name.to_owned());
        if self.fail {
            Err(InfisicalError)
        } else {
            Ok(InfisicalSecret {
                secret_value: self.value.clone(),
            })
        }
    }
}
