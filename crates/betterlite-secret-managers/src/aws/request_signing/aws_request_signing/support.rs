use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aws_credential_types::Credentials;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use crate::aws::request_signing::{
    ActionRequest, AwsCredentialsInfo, AwsOptionalParams, CredentialResolver,
};
use crate::aws::{AwsDependencies, AwsSecretsManager};

pub(super) const ACCESS_KEY: &str = "AKIDEXAMPLE";
pub(super) const SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY";
pub(super) const SESSION_TOKEN: &str = "session-token";

pub(super) fn signing_time() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(1_709_641_845))
        .unwrap()
}

pub(super) fn dns_suffix(region: &str) -> &'static str {
    if region.starts_with("cn-") {
        "amazonaws.com.cn"
    } else {
        "amazonaws.com"
    }
}

#[derive(Debug, Error)]
pub(super) enum ResolverFailure {
    #[error("credentials are unavailable")]
    Unavailable,
}

const TEXT_KEYS: [&str; 7] = [
    "role",
    "session_name",
    "region",
    "external_id",
    "profile",
    "web_identity_token",
    "sts_endpoint",
];

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct Snapshot(pub(super) BTreeMap<&'static str, String>);

impl Snapshot {
    pub(super) fn filled(prefix: &str) -> Self {
        Self(
            TEXT_KEYS
                .into_iter()
                .map(|key| (key, format!("{prefix}-{key}")))
                .collect(),
        )
    }

    pub(super) fn empty_strings() -> Self {
        Self(
            TEXT_KEYS
                .into_iter()
                .map(|key| (key, String::new()))
                .collect(),
        )
    }

    pub(super) fn into_params(mut self) -> AwsOptionalParams {
        let mut take = |key: &str| self.0.remove(key);
        let web_identity_token = take("web_identity_token").map(SecretString::from);
        let access_key_id = take("access_key_id").map(SecretString::from);
        AwsOptionalParams {
            aws_role_name: take("role"),
            aws_session_name: take("session_name"),
            aws_region_name: take("region"),
            aws_external_id: take("external_id"),
            aws_profile_name: take("profile"),
            aws_web_identity_token: web_identity_token,
            aws_sts_endpoint: take("sts_endpoint"),
            aws_access_key_id: access_key_id,
            aws_secret_access_key: None,
            aws_session_token: None,
            aws_bedrock_runtime_endpoint: None,
        }
    }

    pub(super) fn of(params: &AwsOptionalParams) -> Self {
        let secret_text =
            |secret: Option<&SecretString>| secret.map(|value| value.expose_secret().to_owned());
        let present = [
            ("role", params.aws_role_name.clone()),
            ("session_name", params.aws_session_name.clone()),
            ("region", params.aws_region_name.clone()),
            ("external_id", params.aws_external_id.clone()),
            ("profile", params.aws_profile_name.clone()),
            (
                "web_identity_token",
                secret_text(params.aws_web_identity_token.as_ref()),
            ),
            ("sts_endpoint", params.aws_sts_endpoint.clone()),
            (
                "access_key_id",
                secret_text(params.aws_access_key_id.as_ref()),
            ),
        ];
        Self(
            present
                .into_iter()
                .filter_map(|(key, value)| value.map(|text| (key, text)))
                .collect(),
        )
    }
}

#[derive(Debug)]
pub(super) struct FakeResolver {
    pub(super) region: String,
    pub(super) endpoint: Option<String>,
    pub(super) has_session_token: bool,
    pub(super) fail: bool,
    pub(super) seen: Mutex<Vec<Snapshot>>,
}

impl CredentialResolver for FakeResolver {
    type Error = ResolverFailure;

    fn resolve_credentials(
        &self,
        optional_params: &mut AwsOptionalParams,
    ) -> Result<AwsCredentialsInfo, ResolverFailure> {
        self.seen
            .lock()
            .map_err(|_poisoned| ResolverFailure::Unavailable)?
            .push(Snapshot::of(optional_params));
        optional_params.aws_role_name = None;
        if self.fail {
            return Err(ResolverFailure::Unavailable);
        }
        Ok(AwsCredentialsInfo {
            aws_region_name: self.region.clone(),
            aws_bedrock_runtime_endpoint: self.endpoint.clone(),
            credentials: Credentials::new(
                ACCESS_KEY,
                SECRET_KEY,
                self.has_session_token.then(|| SESSION_TOKEN.to_owned()),
                None,
                "test",
            ),
        })
    }
}

pub(super) fn resolver(region: &str) -> FakeResolver {
    FakeResolver {
        region: region.to_owned(),
        endpoint: None,
        has_session_token: true,
        fail: false,
        seen: Mutex::new(Vec::new()),
    }
}

pub(super) fn manager(
    resolver: FakeResolver,
    environment: BTreeMap<String, String>,
) -> AwsSecretsManager<FakeResolver> {
    AwsSecretsManager {
        aws_region_name: None,
        aws_role_name: None,
        aws_session_name: None,
        aws_external_id: None,
        aws_profile_name: None,
        aws_web_identity_token: None,
        aws_sts_endpoint: None,
        replica_regions: Vec::new(),
        kms_key_id: None,
        dependencies: AwsDependencies {
            environment,
            http_client: reqwest::Client::default(),
            resolver,
            dns_suffix,
        },
    }
}

pub(super) fn manager_with_instance_settings(
    resolver: FakeResolver,
    settings: &Snapshot,
) -> AwsSecretsManager<FakeResolver> {
    let mut client = manager(resolver, BTreeMap::new());
    client.aws_role_name = settings.0.get("role").cloned();
    client.aws_session_name = settings.0.get("session_name").cloned();
    client.aws_region_name = settings.0.get("region").cloned();
    client.aws_external_id = settings.0.get("external_id").cloned();
    client.aws_profile_name = settings.0.get("profile").cloned();
    client.aws_web_identity_token = settings
        .0
        .get("web_identity_token")
        .cloned()
        .map(SecretString::from);
    client.aws_sts_endpoint = settings.0.get("sts_endpoint").cloned();
    client
}

pub(super) fn get_secret_request(name: &str) -> ActionRequest<'_> {
    ActionRequest {
        action: "GetSecretValue",
        secret_name: name,
        request_data: None,
    }
}
