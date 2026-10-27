crate::sections::stub_section!();
mod tests;

#[derive(Debug, Clone, serde::Deserialize, PartialEq, Eq)]
#[serde(try_from = "RawWorkerRegistryEntry")]
pub struct WorkerRegistryEntry {
    pub worker_id: String,
    pub name: String,
    pub url: String,
}

#[derive(Debug, serde::Deserialize)]
struct RawWorkerRegistryEntry {
    worker_id: String,
    name: String,
    url: String,
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

impl TryFrom<RawWorkerRegistryEntry> for WorkerRegistryEntry {
    type Error = WorkerRegistryUrlError;

    fn try_from(raw: RawWorkerRegistryEntry) -> Result<Self, Self::Error> {
        Self::validate_url(&raw.url)?;
        Ok(Self {
            worker_id: raw.worker_id,
            name: raw.name,
            url: raw.url,
        })
    }
}
