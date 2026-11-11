use super::chain::{Chain, ChainName, Link, ManagedIdentityIdentity};
use super::environment::ChainEnvironment;
use super::error::DefaultChainError;
use super::grant::{ChainPlacement, Grants, ManagedIdentityRecord};
use super::plan::{construction, non_empty, workload_link, workload_record};

// LiteLLM builds a plain ChainedTokenCredential here, so the workload and both identity links are separate
pub fn deployment_chain<'grants, F>(
    environment: &ChainEnvironment,
    grants: &'grants F,
    scope: &str,
) -> Result<Chain<F::Grant<'grants>>, DefaultChainError>
where
    F: Grants + 'grants,
{
    let mut links = Vec::with_capacity(3);
    if workload_record(environment, scope).is_some() {
        links.push(workload_link(environment, grants, scope)?);
    }
    if let Some(client_id) = non_empty(environment.client_id.as_deref()) {
        let record = ManagedIdentityRecord {
            client_id: Some(client_id.to_owned()),
            exclude_workload_identity: false,
            imds_probe: None,
            placement: ChainPlacement::Chained,
            scope: scope.to_owned(),
        };
        links.push(Link::ManagedIdentity {
            identity: ManagedIdentityIdentity::UserAssigned,
            grant: grants
                .managed_identity(&record)
                .map_err(|error| construction("ManagedIdentityCredential", error))?,
        });
    }
    let system = ManagedIdentityRecord {
        client_id: None,
        exclude_workload_identity: false,
        imds_probe: None,
        placement: ChainPlacement::Chained,
        scope: scope.to_owned(),
    };
    links.push(Link::ManagedIdentity {
        identity: ManagedIdentityIdentity::SystemAssigned,
        grant: grants
            .managed_identity(&system)
            .map_err(|error| construction("ManagedIdentityCredential", error))?,
    });
    Ok(Chain {
        name: ChainName::ChainedTokenCredential,
        links,
        warnings: Vec::new(),
    })
}
