use super::error::{FetchFailure, GitLabClientError};
use super::url::FileEndpoint;

pub(super) const MAX_FILE_BYTES: usize = 0x0010_0000;

// 401 and 403 keep their own messages; everything else reports the endpoint that failed.
pub(super) fn classify(
    endpoint: FileEndpoint,
    project: &str,
    file_path: &str,
    failure: FetchFailure,
) -> GitLabClientError {
    let path = file_path.to_owned();
    match (failure, endpoint) {
        (FetchFailure::Status(401), _) => GitLabClientError::Unauthorized,
        (FetchFailure::Status(403), _) => GitLabClientError::AccessDenied {
            file_path: path,
            project: project.to_owned(),
        },
        (source, FileEndpoint::Raw) => GitLabClientError::Fetch {
            file_path: path,
            source,
        },
        (source, FileEndpoint::Json) => GitLabClientError::JsonFetch {
            file_path: path,
            source,
        },
    }
}

mod tests;
