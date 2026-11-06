use std::path::Path;

use hf_hub::HFError;
use tokio::fs;

use super::HubError;

enum Kind {
    Revision,
    Entry,
    Repo,
    Unauthorized,
    Gated,
    Cache(String),
    Other,
}

fn kind_of(error: &HFError) -> Kind {
    match error {
        HFError::RevisionNotFound { .. } => Kind::Revision,
        HFError::EntryNotFound { .. } => Kind::Entry,
        HFError::RepoNotFound { .. } => Kind::Repo,
        HFError::AuthRequired { .. } => Kind::Unauthorized,
        HFError::Forbidden { context } => {
            if context.body.to_ascii_lowercase().contains("gated") {
                Kind::Gated
            } else {
                Kind::Other
            }
        }
        HFError::LocalEntryNotFound { path } => Kind::Cache(path.clone()),
        HFError::Http { .. }
        | HFError::BucketNotFound { .. }
        | HFError::Conflict { .. }
        | HFError::RateLimited { .. }
        | HFError::CacheNotEnabled
        | HFError::CacheLockTimeout { .. }
        | HFError::Request { .. }
        | HFError::Io(_)
        | HFError::Json(_)
        | HFError::Url(_)
        | HFError::InvalidParameter(_)
        | HFError::DiffParse(_)
        | HFError::Xet { .. }
        | HFError::MalformedResponse { .. }
        | HFError::Other(_)
        | _ => Kind::Other,
    }
}

pub(super) fn classify(error: HFError, repository: &str, revision: &str) -> HubError {
    match kind_of(&error) {
        Kind::Revision => HubError::RevisionNotFound {
            repository: repository.to_owned(),
            revision: revision.to_owned(),
            source: error,
        },
        Kind::Entry => HubError::EntryNotFound {
            repository: repository.to_owned(),
            source: error,
        },
        Kind::Repo => HubError::RepoNotFound {
            repository: repository.to_owned(),
            source: error,
        },
        Kind::Unauthorized => HubError::Unauthorized { source: error },
        Kind::Gated => HubError::Gated {
            repository: repository.to_owned(),
            source: error,
        },
        Kind::Cache(path) => HubError::CacheMiss {
            repository: repository.to_owned(),
            path,
        },
        Kind::Other => HubError::Download {
            repository: repository.to_owned(),
            source: error,
        },
    }
}

pub(super) fn retryable(error: &HubError) -> bool {
    if matches!(error, HubError::StartTimeout { .. }) {
        return true;
    }
    let HubError::Download { source, .. } = error else {
        return false;
    };
    match source {
        HFError::Request {
            source: transport, ..
        } => transport.is_timeout() || transport.is_connect(),
        HFError::Http { context } => {
            matches!(context.status.as_u16(), 408 | 500 | 502 | 503 | 504)
        }
        HFError::RateLimited { .. } => true,
        HFError::AuthRequired { .. }
        | HFError::RepoNotFound { .. }
        | HFError::RevisionNotFound { .. }
        | HFError::EntryNotFound { .. }
        | HFError::BucketNotFound { .. }
        | HFError::Forbidden { .. }
        | HFError::Conflict { .. }
        | HFError::LocalEntryNotFound { .. }
        | HFError::CacheNotEnabled
        | HFError::CacheLockTimeout { .. }
        | HFError::Io(_)
        | HFError::Json(_)
        | HFError::Url(_)
        | HFError::InvalidParameter(_)
        | HFError::DiffParse(_)
        | HFError::Xet { .. }
        | HFError::MalformedResponse { .. }
        | HFError::Other(_)
        | _ => false,
    }
}

pub(super) fn not_available(error: HubError, repository: &str) -> HubError {
    if error.keeps_identity() {
        return error;
    }
    HubError::NotAvailable {
        repository: repository.to_owned(),
        source: Box::new(error),
    }
}

pub(super) async fn read_file(path: &Path) -> Result<Vec<u8>, HubError> {
    fs::read(path).await.map_err(|source| HubError::Read {
        path: path.to_path_buf(),
        source,
    })
}
