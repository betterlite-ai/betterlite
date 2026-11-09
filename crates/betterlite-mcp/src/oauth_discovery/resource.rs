use super::document::{MetadataDocument, ServerEntry, document_scopes};
use super::fetch::{MetadataFetcher, fetch_oauth_discovery_url};
use super::outcome::DiscoveryStatus;
use super::settings::DiscoverySettings;
use crate::oauth_resource::python_url;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResourceMetadata {
    pub authorization_servers: Vec<String>,
    pub scopes_supported: Option<Vec<String>>,
    pub resource: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResourceLookup {
    pub authorization_servers: Vec<String>,
    pub scopes: Option<Vec<String>>,
    pub statuses: Vec<DiscoveryStatus>,
}

pub async fn fetch_resource_metadata<F>(
    fetcher: &F,
    url: &str,
    settings: DiscoverySettings,
) -> Result<ResourceMetadata, DiscoveryStatus>
where
    F: MetadataFetcher,
{
    fetch_oauth_discovery_url(fetcher, url, settings.metadata_timeout)
        .await
        .map(|document| resource_metadata(&document))
}

pub async fn attempt_well_known_discovery<F>(
    fetcher: &F,
    server_url: &str,
    settings: DiscoverySettings,
) -> ResourceLookup
where
    F: MetadataFetcher,
{
    let mut lookup = ResourceLookup::default();
    for url in well_known_candidates(server_url) {
        let resource = match fetch_resource_metadata(fetcher, &url, settings).await {
            Ok(resource) => resource,
            Err(status) => {
                lookup.statuses.push(status);
                continue;
            }
        };
        if resource.authorization_servers.is_empty() {
            continue;
        }
        lookup.authorization_servers = resource.authorization_servers;
        lookup.scopes = resource.scopes_supported;
        return lookup;
    }
    lookup
}

fn resource_metadata(document: &MetadataDocument) -> ResourceMetadata {
    let authorization_servers = document
        .authorization_servers
        .as_deref()
        .map_or_else(Vec::new, |entries| {
            entries.iter().filter_map(server_entry).collect()
        });
    ResourceMetadata {
        authorization_servers,
        scopes_supported: document_scopes(
            document
                .scopes_supported
                .as_ref()
                .or(document.scopes.as_ref()),
        ),
        resource: document.resource.clone(),
    }
}

fn server_entry(entry: &ServerEntry) -> Option<String> {
    match entry {
        ServerEntry::Url(url) if !url.trim().is_empty() => Some(url.clone()),
        ServerEntry::Url(_) | ServerEntry::Other(_) => None,
    }
}

fn well_known_candidates(server_url: &str) -> Vec<String> {
    let Ok(parsed) = python_url::parse(server_url) else {
        return Vec::new();
    };
    if parsed.scheme.is_empty() || parsed.netloc.is_empty() {
        return Vec::new();
    }
    let base = python_url::join(&parsed.scheme, &parsed.netloc, "", "", "");
    let path = parsed.path.trim_matches('/');
    let mut candidates = Vec::new();
    if !path.is_empty() {
        candidates.push(format!(
            "{base}/.well-known/oauth-protected-resource/{path}"
        ));
    }
    candidates.push(format!("{base}/.well-known/oauth-protected-resource"));
    candidates
}
