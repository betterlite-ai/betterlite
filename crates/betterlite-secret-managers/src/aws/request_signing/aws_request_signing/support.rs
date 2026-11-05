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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct Snapshot {
    pub(super) role: Option<String>,
    pub(super) session_name: Option<String>,
    pub(super) region: Option<String>,
    pub(super) external_id: Option<String>,
    pub(super) profile: Option<String>,
    pub(super) web_identity_token: Option<String>,
    pub(super) sts_endpoint: Option<String>,
    pub(super) access_key_id: Option<String>,
}

impl Snapshot {
    pub(super) fn filled(prefix: &str) -> Self {
        let text = |name: &str| Some(format!("{prefix}-{name}"));
        Self {
            role: text("role"),
            session_name: text("session"),
            region: text("region"),
            external_id: text("external"),
            profile: text("profile"),
            web_identity_token: text("token"),
            sts_endpoint: text("sts"),
            access_key_id: None,
        }
    }

    pub(super) fn empty_strings() -> Self {
        let empty = || Some(String::new());
        Self {
            role: empty(),
            session_name: empty(),
            region: empty(),
            external_id: empty(),
            profile: empty(),
            web_identity_token: empty(),
            sts_endpoint: empty(),
            access_key_id: None,
        }
    }

    pub(super) fn into_params(self) -> AwsOptionalParams {
        AwsOptionalParams {
            aws_role_name: self.role,
            aws_session_name: self.session_name,
            aws_region_name: self.region,
            aws_external_id: self.external_id,
            aws_profile_name: self.profile,
            aws_web_identity_token: self.web_identity_token.map(SecretString::from),
            aws_sts_endpoint: self.sts_endpoint,
            aws_access_key_id: self.access_key_id.map(SecretString::from),
            aws_secret_access_key: None,
            aws_session_token: None,
            aws_bedrock_runtime_endpoint: None,
        }
    }

    pub(super) fn of(params: &AwsOptionalParams) -> Self {
        Self {
            role: params.aws_role_name.clone(),
            session_name: params.aws_session_name.clone(),
            region: params.aws_region_name.clone(),
            external_id: params.aws_external_id.clone(),
            profile: params.aws_profile_name.clone(),
            web_identity_token: params
                .aws_web_identity_token
                .as_ref()
                .map(|token| token.expose_secret().to_owned()),
            sts_endpoint: params.aws_sts_endpoint.clone(),
            access_key_id: params
                .aws_access_key_id
                .as_ref()
                .map(|key| key.expose_secret().to_owned()),
        }
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
    client.aws_role_name.clone_from(&settings.role);
    client.aws_session_name.clone_from(&settings.session_name);
    client.aws_region_name.clone_from(&settings.region);
    client.aws_external_id.clone_from(&settings.external_id);
    client.aws_profile_name.clone_from(&settings.profile);
    client.aws_web_identity_token = settings.web_identity_token.clone().map(SecretString::from);
    client.aws_sts_endpoint.clone_from(&settings.sts_endpoint);
    client
}

pub(super) fn get_secret_request(name: &str) -> ActionRequest<'_> {
    ActionRequest {
        action: "GetSecretValue",
        secret_name: name,
        request_data: None,
    }
}
