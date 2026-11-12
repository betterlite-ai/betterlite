use super::chain::{Chain, ChainName, Link};
use super::environment::ChainEnvironment;
use super::error::DefaultChainError;
use super::grant::{ChainPlacement, Grants, ManagedIdentityRecord};
use super::plan::{construction, workload_link, workload_record};

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
    if let Some(client_id) = environment
        .client_id
        .as_deref()
        .filter(|text| !text.is_empty())
    {
        let record = ManagedIdentityRecord {
            client_id: Some(client_id.to_owned()),
            exclude_workload_identity: false,
            imds_probe: None,
            placement: ChainPlacement::Chained,
            scope: scope.to_owned(),
        };
        links.push(managed_link(&record, grants)?);
    }
    let system = ManagedIdentityRecord {
        client_id: None,
        exclude_workload_identity: false,
        imds_probe: None,
        placement: ChainPlacement::Chained,
        scope: scope.to_owned(),
    };
    links.push(managed_link(&system, grants)?);
    Ok(Chain {
        name: ChainName::ChainedTokenCredential,
        links,
        warnings: Vec::new(),
    })
}

fn managed_link<'grants, F>(
    record: &ManagedIdentityRecord,
    grants: &'grants F,
) -> Result<Link<F::Grant<'grants>>, DefaultChainError>
where
    F: Grants + 'grants,
{
    Ok(Link::ManagedIdentity(
        grants.managed_identity(record).map_err(construction)?,
    ))
}
