use thiserror::Error;

#[derive(Debug, Error)]
pub enum BootRegistryError {
    #[error("{key} is set in the config, but its support is not built yet")]
    NotBuilt { key: &'static str },
}
