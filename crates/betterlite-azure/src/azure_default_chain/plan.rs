use super::chain::{Chain, ChainName, Link, LinkInner, ManagedIdentityIdentity};
use super::environment::ChainEnvironment;
use super::error::{DefaultChainError, ENVIRONMENT_UNAVAILABLE, workload_missing_message};
use super::grant::{
    CertificateRecord, ChainPlacement, ClientSecretRecord, GrantError, Grants,
    ManagedIdentityRecord, UsernamePasswordRecord, WorkloadRecord,
};
use super::selection::{Selection, selection};

pub const ENVIRONMENT_PASSWORD_WARNING: &str = "Environment is configured to use username and password authentication. This authentication method is deprecated, as it doesn't support multifactor authentication (MFA). For more details, see https://aka.ms/azsdk/identity/mfa.";

#[must_use]
pub fn construction(credential: &'static str, error: GrantError) -> DefaultChainError {
    DefaultChainError::Construction {
        credential,
        message: error.message,
    }
}

#[must_use]
pub const fn identity(client_id: Option<&str>) -> ManagedIdentityIdentity {
    match client_id {
        Some(text) if !text.is_empty() => ManagedIdentityIdentity::UserAssigned,
        Some(_) | None => ManagedIdentityIdentity::SystemAssigned,
    }
}

#[must_use]
pub fn missing_workload_arguments(environment: &ChainEnvironment) -> Vec<&'static str> {
    [
        ("tenant_id", environment.tenant_id.as_deref()),
        ("client_id", environment.client_id.as_deref()),
        (
            "token_file_path",
            environment.federated_token_file.as_deref(),
        ),
    ]
    .into_iter()
    .filter(|(_, value)| value.as_ref().is_none_or(|text| text.is_empty()))
    .map(|(name, _)| name)
    .collect()
}

#[must_use]
pub fn workload_record(environment: &ChainEnvironment, scope: &str) -> Option<WorkloadRecord> {
    Some(WorkloadRecord {
        client_id: environment
            .client_id
            .as_deref()
            .filter(|text| !text.is_empty())?
            .to_owned(),
        tenant_id: environment
            .tenant_id
            .as_deref()
            .filter(|text| !text.is_empty())?
            .to_owned(),
        token_file: environment
            .federated_token_file
            .as_deref()
            .filter(|text| !text.is_empty())?
            .to_owned(),
        scope: scope.to_owned(),
    })
}

// EnvironmentCredential tests presence, so an empty variable still selects its branch
fn secret_grant<'grants, F>(
    environment: &ChainEnvironment,
    grants: &'grants F,
    scope: &str,
) -> Result<Option<F::Grant<'grants>>, DefaultChainError>
where
    F: Grants + 'grants,
{
    let (Some(client_id), Some(client_secret), Some(tenant_id)) = (
        &environment.client_id,
        &environment.client_secret,
        &environment.tenant_id,
    ) else {
        return Ok(None);
    };
    let record = ClientSecretRecord {
        tenant_id: tenant_id.clone(),
        client_id: client_id.clone(),
        client_secret: client_secret.clone(),
        scope: scope.to_owned(),
    };
    grants
        .client_secret(&record)
        .map(Some)
        .map_err(|error| construction("ClientSecretCredential", error))
}

fn certificate_grant<'grants, F>(
    environment: &ChainEnvironment,
    grants: &'grants F,
    scope: &str,
) -> Result<Option<F::Grant<'grants>>, DefaultChainError>
where
    F: Grants + 'grants,
{
    let (Some(client_id), Some(certificate_path), Some(tenant_id)) = (
        &environment.client_id,
        &environment.client_certificate_path,
        &environment.tenant_id,
    ) else {
        return Ok(None);
    };
    let record = CertificateRecord {
        tenant_id: tenant_id.clone(),
        client_id: client_id.clone(),
        certificate_path: certificate_path.clone(),
        password: environment.client_certificate_password.clone(),
        send_certificate_chain: environment.client_send_certificate_chain,
        scope: scope.to_owned(),
    };
    grants
        .certificate(&record)
        .map(Some)
        .map_err(|error| construction("CertificateCredential", error))
}

fn password_grant<'grants, F>(
    environment: &ChainEnvironment,
    grants: &'grants F,
    warnings: &mut Vec<String>,
    scope: &str,
) -> Result<Option<F::Grant<'grants>>, DefaultChainError>
where
    F: Grants + 'grants,
{
    let (Some(client_id), Some(username), Some(password)) = (
        &environment.client_id,
        &environment.username,
        &environment.password,
    ) else {
        return Ok(None);
    };
    warnings.push(ENVIRONMENT_PASSWORD_WARNING.to_owned());
    let record = UsernamePasswordRecord {
        client_id: client_id.clone(),
        username: username.clone(),
        password: password.clone(),
        tenant_id: environment.tenant_id.clone(),
        scope: scope.to_owned(),
    };
    grants
        .username_password(&record)
        .map(Some)
        .map_err(|error| construction("UsernamePasswordCredential", error))
}

fn environment_grant<'grants, F>(
    environment: &ChainEnvironment,
    grants: &'grants F,
    warnings: &mut Vec<String>,
    scope: &str,
) -> Result<Option<F::Grant<'grants>>, DefaultChainError>
where
    F: Grants + 'grants,
{
    if let Some(grant) = secret_grant(environment, grants, scope)? {
        return Ok(Some(grant));
    }
    if let Some(grant) = certificate_grant(environment, grants, scope)? {
        return Ok(Some(grant));
    }
    password_grant(environment, grants, warnings, scope)
}

fn environment_link<'grants, F>(
    environment: &ChainEnvironment,
    grants: &'grants F,
    warnings: &mut Vec<String>,
    scope: &str,
) -> Result<Link<F::Grant<'grants>>, DefaultChainError>
where
    F: Grants + 'grants,
{
    let inner = environment_grant(environment, grants, warnings, scope)?.map_or_else(
        || LinkInner::Unavailable(ENVIRONMENT_UNAVAILABLE.to_owned()),
        LinkInner::Ready,
    );
    Ok(Link::Environment(inner))
}

pub fn workload_link<'grants, F>(
    environment: &ChainEnvironment,
    grants: &'grants F,
    scope: &str,
) -> Result<Link<F::Grant<'grants>>, DefaultChainError>
where
    F: Grants + 'grants,
{
    let inner = match workload_record(environment, scope) {
        Some(record) => LinkInner::Ready(
            grants
                .workload(&record)
                .map_err(|error| construction("WorkloadIdentityCredential", error))?,
        ),
        None => LinkInner::Unavailable(workload_missing_message(&missing_workload_arguments(
            environment,
        ))),
    };
    Ok(Link::Workload(inner))
}

fn managed_link<'grants, F>(
    environment: &ChainEnvironment,
    grants: &'grants F,
    selected: Selection,
    scope: &str,
) -> Result<Link<F::Grant<'grants>>, DefaultChainError>
where
    F: Grants + 'grants,
{
    let record = ManagedIdentityRecord {
        client_id: environment.client_id.clone(),
        exclude_workload_identity: !selected.includes_workload(),
        imds_probe: Some(environment.token_credentials != "managedidentitycredential"),
        placement: ChainPlacement::DefaultCredential,
        scope: scope.to_owned(),
    };
    Ok(Link::ManagedIdentity {
        identity: identity(record.client_id.as_deref()),
        grant: grants
            .managed_identity(&record)
            .map_err(|error| construction("ManagedIdentityCredential", error))?,
    })
}

pub fn default_chain<'grants, F>(
    environment: &ChainEnvironment,
    grants: &'grants F,
    scope: &str,
) -> Result<Chain<F::Grant<'grants>>, DefaultChainError>
where
    F: Grants + 'grants,
{
    let selected = selection(&environment.token_credentials)?;
    let mut links = Vec::with_capacity(3);
    let mut warnings = Vec::new();
    if selected.includes_environment() {
        links.push(environment_link(environment, grants, &mut warnings, scope)?);
    }
    if selected.includes_workload() {
        links.push(workload_link(environment, grants, scope)?);
    }
    if selected.includes_managed() {
        links.push(managed_link(environment, grants, selected, scope)?);
    }
    Ok(Chain {
        name: ChainName::DefaultAzureCredential,
        links,
        warnings,
    })
}
