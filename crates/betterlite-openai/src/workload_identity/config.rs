use betterlite_config::sections::openai_credentials::Settings as OpenAiCredentialsSettings;
use betterlite_config::sections::openai_workload_identity::Settings as IdentitySettings;
use betterlite_secret_managers::secret_helpers::normalize_nonempty_secret_str;
use secrecy::SecretString;

use super::error::{SdkImportError, WorkloadIdentityError};
use crate::credentials::{is_openai_backed_api_base, resolve_openai_api_base};
use crate::error::{ProviderError, ProviderErrorParts};

pub const OPENAI_WIF_CLIENT_ID: &str = "litellm";
const SDK_UPGRADE_MESSAGE: &str = "OpenAI workload identity federation requires openai>=2.32.0. Upgrade the installed openai package to use OPENAI_IDENTITY_PROVIDER_ID / OPENAI_SERVICE_ACCOUNT_ID / OPENAI_IDENTITY_TOKEN_FILE.";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OpenAiWorkloadIdentityConfig {
    pub identity_provider_id: String,
    pub service_account_id: String,
    pub token_file: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SdkWorkloadIdentity {
    pub client_id: &'static str,
    pub identity_provider_id: String,
    pub service_account_id: String,
    pub token_file: String,
}

#[derive(Debug, Clone, Copy)]
pub struct SdkCapability {
    pub workload_identity_auth: bool,
    pub k8s_token_provider: bool,
}

impl SdkCapability {
    pub const AVAILABLE: Self = Self {
        workload_identity_auth: true,
        k8s_token_provider: true,
    };

    pub fn load_workload_identity_auth(self) -> Result<(), WorkloadIdentityError> {
        if self.workload_identity_auth {
            return Ok(());
        }
        Err(sdk_unavailable(SdkImportError::WorkloadIdentityAuth))
    }

    pub fn load_k8s_token_provider(self) -> Result<(), WorkloadIdentityError> {
        if self.k8s_token_provider {
            return Ok(());
        }
        Err(sdk_unavailable(SdkImportError::K8sTokenProvider))
    }
}

fn sdk_unavailable(cause: SdkImportError) -> WorkloadIdentityError {
    let provider = ProviderError::from(ProviderErrorParts {
        status_code: 500,
        message: SDK_UPGRADE_MESSAGE.to_owned(),
        headers: None,
        request: None,
        response: None,
        body: None,
        status_code_is_synthesized: false,
    });
    WorkloadIdentityError::SdkUnavailable {
        provider: Box::new(provider),
        cause,
    }
}

impl OpenAiWorkloadIdentityConfig {
    pub fn to_sdk_workload_identity(
        &self,
        sdk: SdkCapability,
    ) -> Result<SdkWorkloadIdentity, WorkloadIdentityError> {
        sdk.load_k8s_token_provider()?;
        Ok(SdkWorkloadIdentity {
            client_id: OPENAI_WIF_CLIENT_ID,
            identity_provider_id: self.identity_provider_id.clone(),
            service_account_id: self.service_account_id.clone(),
            token_file: self.token_file.clone(),
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct WorkloadIdentityEnvironment<'settings> {
    pub env_api_key: Option<&'settings SecretString>,
    pub litellm_api_base: Option<&'settings str>,
    pub credentials: &'settings OpenAiCredentialsSettings,
    pub identity: &'settings IdentitySettings,
}

pub fn resolve_config(
    api_key: Option<&SecretString>,
    api_base: Option<&str>,
    environment: WorkloadIdentityEnvironment<'_>,
) -> Result<Option<OpenAiWorkloadIdentityConfig>, WorkloadIdentityError> {
    let static_api_key = normalize_nonempty_secret_str(api_key)
        .or_else(|| normalize_nonempty_secret_str(environment.env_api_key));
    if static_api_key.is_some() {
        return Ok(None);
    }
    let effective_api_base = resolve_openai_api_base(
        api_base,
        environment.litellm_api_base,
        environment.credentials,
    );
    if !targets_openai_api(&effective_api_base)? {
        return Ok(None);
    }
    let identity = environment.identity;
    let (Some(identity_provider_id), Some(service_account_id), Some(token_file)) = (
        identity.identity_provider_id.as_ref(),
        identity.service_account_id.as_ref(),
        identity.token_file.as_ref(),
    ) else {
        return Ok(None);
    };
    Ok(Some(OpenAiWorkloadIdentityConfig {
        identity_provider_id: identity_provider_id.clone(),
        service_account_id: service_account_id.clone(),
        token_file: token_file.clone(),
    }))
}

// An absent base is the default https://api.openai.com/v1 by the time it reaches here, which passes both checks.
pub fn targets_openai_api(api_base: &str) -> Result<bool, WorkloadIdentityError> {
    let openai_backed =
        is_openai_backed_api_base(api_base).map_err(WorkloadIdentityError::ApiBase)?;
    Ok(has_https_scheme(api_base) && openai_backed)
}

// urlparse(...).scheme == "https": urlsplit lstrips C0 controls and spaces, drops tab, CR and LF, then lowercases the scheme
fn has_https_scheme(api_base: &str) -> bool {
    let sanitized: String = api_base
        .trim_start_matches(|letter: char| letter <= '\u{0020}')
        .chars()
        .filter(|letter| !matches!(letter, '\t' | '\r' | '\n'))
        .collect();
    sanitized
        .get(..6)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("https:"))
}
