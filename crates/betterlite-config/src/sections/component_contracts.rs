crate::sections::stub_section!();
mod tests;

#[derive(Debug, Clone, serde::Deserialize, PartialEq, Eq)]
pub struct WorkerRegistryEntry {
    pub worker_id: String,
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("Worker URL must start with http:// or https://")]
pub struct WorkerRegistryUrlError;

impl WorkerRegistryEntry {
    pub fn validate_url(url: &str) -> Result<(), WorkerRegistryUrlError> {
        if url.starts_with("http://") || url.starts_with("https://") {
            Ok(())
        } else {
            Err(WorkerRegistryUrlError)
        }
    }
}
