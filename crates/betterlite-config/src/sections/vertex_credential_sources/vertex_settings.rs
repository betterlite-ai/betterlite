use std::path::PathBuf;

// the inputs the Google auth SDK documents for application default credentials, resolved here
#[derive(Debug, Clone, Default)]
pub struct AdcEnvironment {
    pub application_credentials: Option<PathBuf>,
    pub project: Option<String>,
    pub gcloud_project: Option<String>,
    pub gcloud_credentials: Option<PathBuf>,
    pub gcloud_configuration: Option<PathBuf>,
    pub metadata_host: Option<String>,
}

mod tests;
