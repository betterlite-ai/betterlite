use super::HubError;
use hf_hub::HFError;
use std::path::Path;
use tokio::fs;
pub(super) fn classify(error: HFError, repository: &str, revision: &str) -> HubError {
    if matches!(&error, HFError::RevisionNotFound { .. }) {
        return HubError::RevisionNotFound(repository.to_owned(), revision.to_owned(), error);
    }
    if matches!(&error, HFError::EntryNotFound { .. }) {
        return HubError::EntryNotFound(repository.to_owned(), error);
    }
    if matches!(&error, HFError::RepoNotFound { .. }) {
        return HubError::RepoNotFound(repository.to_owned(), error);
    }
    if matches!(&error, HFError::AuthRequired { .. }) {
        return HubError::Unauthorized(error);
    }
    let gated = matches!(&error, HFError::Forbidden { context } if context.body.to_ascii_lowercase().contains("gated"));
    if gated {
        return HubError::Gated(repository.to_owned(), error);
    }
    if let HFError::LocalEntryNotFound { path } = error {
        return HubError::CacheMiss(repository.to_owned(), path);
    }
    HubError::Download(repository.to_owned(), error)
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
    if error.identity() {
        return error;
    }
    HubError::NotAvailable(repository.to_owned(), Box::new(error))
}
pub(super) async fn read_file(path: &Path) -> Result<Vec<u8>, HubError> {
    fs::read(path)
        .await
        .map_err(|source| HubError::Read(path.to_path_buf(), source))
}
