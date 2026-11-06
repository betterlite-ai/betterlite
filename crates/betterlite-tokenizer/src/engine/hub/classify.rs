use super::HubError;
use hf_hub::{HFClient, HFError, split_id};
use std::path::Path;
use tokio::fs;
pub(super) async fn classify(
    client: &HFClient,
    error: HFError,
    repository: &str,
    revision: &str,
) -> HubError {
    if matches!(&error, HFError::RevisionNotFound { .. }) {
        return HubError::RevisionNotFound(repository.to_owned(), revision.to_owned(), error);
    }
    if let HFError::EntryNotFound { context, .. } = &error {
        let code = context
            .as_ref()
            .and_then(|found| found.error_code.as_deref());
        if code == Some("RepoNotFound") {
            return HubError::RepoNotFound(repository.to_owned(), error);
        }
        if code == Some("RevisionNotFound") {
            return HubError::RevisionNotFound(repository.to_owned(), revision.to_owned(), error);
        }
        if code == Some("EntryNotFound") {
            return HubError::EntryNotFound(repository.to_owned(), error);
        }
        return missing_entry(client, error, repository).await;
    }
    if matches!(&error, HFError::RepoNotFound { .. }) {
        return HubError::RepoNotFound(repository.to_owned(), error);
    }
    let gated = matches!(
        &error,
        HFError::AuthRequired { context } | HFError::Forbidden { context }
            if context.error_code.as_deref() == Some("GatedRepo")
    );
    if gated {
        return HubError::Gated(repository.to_owned(), error);
    }
    if matches!(&error, HFError::AuthRequired { .. }) {
        return HubError::Unauthorized(error);
    }
    if let HFError::LocalEntryNotFound { path } = error {
        return HubError::CacheMiss(repository.to_owned(), path);
    }
    HubError::Download(repository.to_owned(), error)
}
async fn missing_entry(client: &HFClient, error: HFError, repository: &str) -> HubError {
    let (owner, name) = split_id(repository);
    let exists = client
        .model(owner.to_owned(), name.to_owned())
        .exists()
        .send()
        .await;
    match exists {
        Ok(false) => HubError::RepoNotFound(repository.to_owned(), error),
        _ => HubError::EntryNotFound(repository.to_owned(), error),
    }
}
pub(super) fn retryable(error: &HubError) -> bool {
    let HubError::Download(_, source) = error else {
        return matches!(error, HubError::StartTimeout(_));
    };
    if let HFError::Request {
        source: transport, ..
    } = source
    {
        return transport.is_timeout() || transport.is_connect();
    }
    if let HFError::Http { context } = source {
        return matches!(context.status.as_u16(), 408 | 500 | 502 | 503 | 504);
    }
    matches!(source, HFError::RateLimited { .. })
}
pub(super) fn not_available(error: HubError, repository: &str) -> HubError {
    if matches!(
        &error,
        HubError::RepoNotFound(..) | HubError::Gated(..) | HubError::Unauthorized(..)
    ) {
        return error;
    }
    HubError::NotAvailable(repository.to_owned(), Box::new(error))
}
pub(super) async fn read_file(path: &Path) -> Result<Vec<u8>, HubError> {
    fs::read(path)
        .await
        .map_err(|source| HubError::Read(path.to_path_buf(), source))
}
