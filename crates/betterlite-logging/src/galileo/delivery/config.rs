use betterlite_config::loader::EnvironmentLookup;
use secrecy::SecretString;

pub const CLOUD_API_BASE_URL: &str = "https://api.galileo.ai";
pub const MAX_IN_MEMORY_RECORDS: usize = 1000;

#[derive(Clone, Debug)]
pub struct GalileoConfig {
    pub api_key: Option<SecretString>,
    pub project_id: Option<String>,
    pub log_stream_id: Option<String>,
    pub username: Option<String>,
    pub password: Option<SecretString>,
    pub base_url: Option<String>,
    pub batch_size: usize,
    pub use_v2_api: bool,
}

impl GalileoConfig {
    // An empty variable counts as unset, the way GalileoObserve.__init__ treats a falsy os.getenv result.
    #[must_use]
    pub fn from_environment(environment: &dyn EnvironmentLookup) -> Self {
        let read = |name: &str| environment.lookup(name).filter(|value| !value.is_empty());
        let api_key = read("GALILEO_API_KEY").map(SecretString::from);
        let base_url = read("GALILEO_BASE_URL")
            .map(|url| url.trim_end_matches('/').to_owned())
            .filter(|url| !url.is_empty())
            .or_else(|| api_key.as_ref().map(|_| CLOUD_API_BASE_URL.to_owned()));
        Self {
            use_v2_api: api_key.is_some(),
            api_key,
            project_id: read("GALILEO_PROJECT_ID"),
            log_stream_id: read("GALILEO_LOG_STREAM_ID"),
            username: read("GALILEO_USERNAME"),
            password: read("GALILEO_PASSWORD").map(SecretString::from),
            base_url,
            batch_size: 1,
        }
    }

    #[must_use]
    pub const fn is_configured(&self) -> bool {
        if self.project_id.is_none() || self.base_url.is_none() {
            return false;
        }
        if self.use_v2_api {
            return true;
        }
        self.username.is_some() && self.password.is_some()
    }
}
