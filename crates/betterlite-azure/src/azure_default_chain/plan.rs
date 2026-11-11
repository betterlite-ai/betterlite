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

enum EnvironmentRecord {
    Secret(ClientSecretRecord),
    Certificate(CertificateRecord),
    Password(UsernamePasswordRecord),
}

impl EnvironmentRecord {
    const fn credential(&self) -> &'static str {
        match self {
            Self::Secret(_) => "ClientSecretCredential",
            Self::Certificate(_) => "CertificateCredential",
            Self::Password(_) => "UsernamePasswordCredential",
        }
    }
}

// EnvironmentCredential tests presence, so an empty variable still selects its branch
fn environment_record(environment: &ChainEnvironment, scope: &str) -> Option<EnvironmentRecord> {
    if let (Some(client_id), Some(client_secret), Some(tenant_id)) = (
        &environment.client_id,
        &environment.client_secret,
        &environment.tenant_id,
    ) {
        Some(EnvironmentRecord::Secret(ClientSecretRecord {
            tenant_id: tenant_id.clone(),
            client_id: client_id.clone(),
            client_secret: client_secret.clone(),
            scope: scope.to_owned(),
        }))
    } else if let (Some(client_id), Some(certificate_path), Some(tenant_id)) = (
        &environment.client_id,
        &environment.client_certificate_path,
        &environment.tenant_id,
    ) {
        Some(EnvironmentRecord::Certificate(CertificateRecord {
            tenant_id: tenant_id.clone(),
            client_id: client_id.clone(),
            certificate_path: certificate_path.clone(),
            password: environment.client_certificate_password.clone(),
            send_certificate_chain: environment.client_send_certificate_chain,
            scope: scope.to_owned(),
        }))
    } else if let (Some(client_id), Some(username), Some(password)) = (
        &environment.client_id,
        &environment.username,
        &environment.password,
    ) {
        Some(EnvironmentRecord::Password(UsernamePasswordRecord {
            client_id: client_id.clone(),
            username: username.clone(),
            password: password.clone(),
            tenant_id: environment.tenant_id.clone(),
            scope: scope.to_owned(),
        }))
    } else {
        None
    }
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
    let Some(record) = environment_record(environment, scope) else {
        return Ok(Link::Environment(LinkInner::Unavailable(
            ENVIRONMENT_UNAVAILABLE.to_owned(),
        )));
    };
    if matches!(record, EnvironmentRecord::Password(_)) {
        warnings.push(ENVIRONMENT_PASSWORD_WARNING.to_owned());
    }
    let inner = match &record {
        EnvironmentRecord::Secret(record) => grants.client_secret(record),
        EnvironmentRecord::Certificate(record) => grants.certificate(record),
        EnvironmentRecord::Password(record) => grants.username_password(record),
    };
    let inner = match inner {
        Ok(inner) => LinkInner::Ready(inner),
        Err(error) => return Err(construction(record.credential(), error)),
    };
    Ok(Link::Environment(inner))
}

// the missing-argument list and the record come from the same three variables, resolved with different policies
fn workload_arguments(environment: &ChainEnvironment) -> Option<(&str, &str, &str)> {
    let client_id = environment
        .client_id
        .as_deref()
        .filter(|text| !text.is_empty())?;
    let tenant_id = environment
        .tenant_id
        .as_deref()
        .filter(|text| !text.is_empty())?;
    let token_file = environment
        .federated_token_file
        .as_deref()
        .filter(|text| !text.is_empty())?;
    Some((client_id, tenant_id, token_file))
}

#[must_use]
pub fn workload_record(environment: &ChainEnvironment, scope: &str) -> Option<WorkloadRecord> {
    let (client_id, tenant_id, token_file) = workload_arguments(environment)?;
    Some(WorkloadRecord {
        client_id: client_id.to_owned(),
        tenant_id: tenant_id.to_owned(),
        token_file: token_file.to_owned(),
        scope: scope.to_owned(),
    })
}

#[must_use]
pub fn missing_workload_arguments(environment: &ChainEnvironment) -> Vec<&'static str> {
    let mut missing = Vec::new();
    if environment
        .tenant_id
        .as_deref()
        .is_none_or(|text| text.is_empty())
    {
        missing.push("tenant_id");
    }
    if environment
        .client_id
        .as_deref()
        .is_none_or(|text| text.is_empty())
    {
        missing.push("client_id");
    }
    if environment
        .federated_token_file
        .as_deref()
        .is_none_or(|text| text.is_empty())
    {
        missing.push("token_file_path");
    }
    missing
}

pub fn workload_link<'grants, F>(
    environment: &ChainEnvironment,
    grants: &'grants F,
    scope: &str,
) -> Result<Link<F::Grant<'grants>>, DefaultChainError>
where
    F: Grants + 'grants,
{
    match workload_record(environment, scope) {
        Some(record) => Ok(Link::Workload(LinkInner::Ready(
            grants
                .workload(&record)
                .map_err(|error| construction("WorkloadIdentityCredential", error))?,
        ))),
        None => Ok(Link::Workload(LinkInner::Unavailable(
            workload_missing_message(&missing_workload_arguments(environment)),
        ))),
    }
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
        let record = ManagedIdentityRecord {
            client_id: environment.client_id.clone(),
            exclude_workload_identity: !selected.includes_workload(),
            imds_probe: Some(environment.token_credentials != "managedidentitycredential"),
            placement: ChainPlacement::DefaultCredential,
            scope: scope.to_owned(),
        };
        let grant = grants
            .managed_identity(&record)
            .map_err(|error| construction("ManagedIdentityCredential", error))?;
        links.push(Link::ManagedIdentity {
            identity: identity(record.client_id.as_deref()),
            grant,
        });
    }
    Ok(Chain {
        name: ChainName::DefaultAzureCredential,
        links,
        warnings,
    })
}
