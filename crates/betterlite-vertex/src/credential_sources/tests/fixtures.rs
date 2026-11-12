#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::error::CredentialError as BedrockCredentialError;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_config::sections::vertex_credential_sources::vertex_settings::AdcEnvironment;
use secrecy::SecretString;

use crate::aws_federation::AwsCredentialsResolver;
use crate::aws_federation::FederationInput;
use crate::credential_sources::adc::AdcDeps;
use crate::credential_sources::aws::AwsCredentialsSource;
use crate::credential_sources::error::CredentialSourceError;
use crate::credential_sources::files::FileReader;
use crate::credential_sources::handles::{LoadDeps, LoadedAuth, load_auth};
use crate::credential_sources::sdk::{
    BuiltSource, CredentialBuilders, CredentialClock, TokenSource,
};

pub(crate) const TOKEN: &str = "ya29.fixture-token";

pub(crate) struct FakeSource {
    pub(crate) fail: bool,
}

impl TokenSource for FakeSource {
    fn access_token(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<SecretString, CredentialSourceError>> + Send + '_>>
    {
        Box::pin(async move {
            if self.fail {
                return Err(CredentialSourceError::Read {
                    path: PathBuf::from("refresh"),
                    source: io::Error::from(io::ErrorKind::TimedOut),
                });
            }
            Ok(SecretString::from(TOKEN.to_owned()))
        })
    }
}

#[derive(Default)]
pub(crate) struct RecordingBuilders {
    pub(crate) calls: Mutex<Vec<String>>,
    pub(crate) fail_refresh: bool,
    pub(crate) metadata_project: Option<String>,
}

impl RecordingBuilders {
    pub(crate) fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn record(&self, call: String) {
        self.calls.lock().unwrap().push(call);
    }

    fn source(&self) -> Arc<dyn TokenSource> {
        Arc::new(FakeSource {
            fail: self.fail_refresh,
        })
    }

    fn recorded_source(&self, call: String) -> Arc<dyn TokenSource> {
        self.record(call);
        self.source()
    }
}

impl CredentialBuilders for RecordingBuilders {
    fn service_account(&self, document: &str, scopes: &[String]) -> BuiltSource {
        Ok(self.recorded_source(format!("service_account {document} {scopes:?}")))
    }

    fn authorized_user(&self, document: &str, scopes: &[String]) -> BuiltSource {
        Ok(self.recorded_source(format!("authorized_user {document} {scopes:?}")))
    }

    fn identity_pool(&self, document: &str, scopes: &[String]) -> BuiltSource {
        Ok(self.recorded_source(format!("identity_pool {document} {scopes:?}")))
    }

    fn identity_pool_with_aws(&self, document: &str, scopes: &[String]) -> BuiltSource {
        Ok(self.recorded_source(format!("identity_pool_with_aws {document} {scopes:?}")))
    }

    fn metadata(&self, endpoint: Option<&str>, scopes: &[String]) -> BuiltSource {
        Ok(self.recorded_source(format!("metadata {endpoint:?} {scopes:?}")))
    }

    fn aws_federation(
        &self,
        input: FederationInput,
        _resolver: Arc<dyn AwsCredentialsSource>,
        _clock: Arc<dyn CredentialClock>,
    ) -> BuiltSource {
        self.record(format!(
            "aws_federation {:?} {:?}",
            input.token_url, input.aws.region_name
        ));
        Ok(self.source())
    }

    fn metadata_project(
        &self,
        endpoint: Option<&str>,
    ) -> Pin<Box<dyn Future<Output = Option<String>> + Send + '_>> {
        self.record(format!("metadata_project {endpoint:?}"));
        Box::pin(ready(self.metadata_project.clone()))
    }
}

#[derive(Default)]
pub(crate) struct FakeFiles {
    pub(crate) contents: BTreeMap<PathBuf, Vec<u8>>,
    pub(crate) denied: BTreeSet<PathBuf>,
}

impl FakeFiles {
    pub(crate) fn with(mut self, path: &str, bytes: &[u8]) -> Self {
        self.contents.insert(PathBuf::from(path), bytes.to_vec());
        self
    }

    pub(crate) fn denying(mut self, path: &str) -> Self {
        self.denied.insert(PathBuf::from(path));
        self
    }
}

impl FileReader for FakeFiles {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        if self.denied.contains(path) {
            return Err(io::Error::from(io::ErrorKind::PermissionDenied));
        }
        self.contents
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }
}

pub(crate) struct NoAwsCredentials;

impl AwsCredentialsResolver for NoAwsCredentials {
    fn credentials(
        &self,
        _settings: &AwsSettings,
    ) -> impl Future<Output = Result<Credentials, BedrockCredentialError>> + Send {
        ready(Err(BedrockCredentialError::MissingCredentials))
    }
}

pub(crate) struct FixedClock;

impl CredentialClock for FixedClock {
    fn now(&self) -> SystemTime {
        UNIX_EPOCH
            .checked_add(Duration::from_hours(490_896))
            .unwrap()
    }
}

pub(crate) struct Fixture {
    pub(crate) builders: RecordingBuilders,
    pub(crate) files: FakeFiles,
    pub(crate) environment: AdcEnvironment,
}

impl Fixture {
    pub(crate) fn new() -> Self {
        Self {
            builders: RecordingBuilders::default(),
            files: FakeFiles::default(),
            environment: AdcEnvironment::default(),
        }
    }

    pub(crate) fn deps(&self) -> LoadDeps<'_> {
        LoadDeps {
            environment: &self.environment,
            files: &self.files,
            http: &self.builders,
            aws: Arc::new(NoAwsCredentials),
            clock: Arc::new(FixedClock),
        }
    }

    pub(crate) fn adc_deps(&self) -> AdcDeps<'_> {
        AdcDeps {
            environment: &self.environment,
            files: &self.files,
            http: &self.builders,
        }
    }

    pub(crate) async fn load(
        &self,
        credentials: Option<&ConfiguredCredentials>,
        project: Option<&str>,
    ) -> Result<LoadedAuth, CredentialSourceError> {
        load_auth(credentials, project, &self.deps()).await
    }
}

pub(crate) fn text(document: &str) -> ConfiguredCredentials {
    ConfiguredCredentials::Text(SecretString::from(document))
}
