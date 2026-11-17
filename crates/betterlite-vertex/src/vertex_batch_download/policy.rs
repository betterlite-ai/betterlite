use crate::vertex_batch_rows::paths::is_passthrough_managed_gcs_url;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadPolicy {
    Original,
    Convert,
}

// the module switch and the managed passthrough prefix both hand the download back untouched
#[must_use]
pub fn download_policy(disabled: bool, request_url: &str) -> DownloadPolicy {
    if disabled || is_passthrough_managed_gcs_url(request_url) {
        return DownloadPolicy::Original;
    }
    DownloadPolicy::Convert
}
