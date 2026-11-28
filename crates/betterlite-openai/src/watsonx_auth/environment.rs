use betterlite_config::loader::EnvironmentLookup;
use secrecy::SecretString;

#[derive(Debug, Default)]
pub struct WatsonxEnvironment {
    pub watsonx_iam_url: Option<String>,
    pub wx_api_key: Option<SecretString>,
    pub watsonx_api_key: Option<SecretString>,
    pub watsonx_apikey: Option<SecretString>,
    pub watsonx_zenapikey: Option<SecretString>,
    pub watsonx_token: Option<SecretString>,
    pub watsonx_api_base: Option<String>,
    pub watsonx_url: Option<String>,
    pub wx_url: Option<String>,
    pub wml_url: Option<String>,
    pub watsonx_project_id: Option<String>,
    pub wx_project_id: Option<String>,
    pub project_id: Option<String>,
    pub watsonx_region: Option<String>,
    pub wx_region: Option<String>,
    pub region: Option<String>,
    pub watsonx_deployment_space_id: Option<String>,
    pub watsonx_space_id: Option<String>,
    pub wx_space_id: Option<String>,
    pub space_id: Option<String>,
}

impl WatsonxEnvironment {
    // LiteLLM reads each name with os.getenv, so an empty value counts as unset
    #[must_use]
    pub fn from_lookup(lookup: &dyn EnvironmentLookup) -> Self {
        let text = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        let secret = |name: &str| text(name).map(SecretString::from);
        Self {
            watsonx_iam_url: text("WATSONX_IAM_URL"),
            wx_api_key: secret("WX_API_KEY"),
            watsonx_api_key: secret("WATSONX_API_KEY"),
            watsonx_apikey: secret("WATSONX_APIKEY"),
            watsonx_zenapikey: secret("WATSONX_ZENAPIKEY"),
            watsonx_token: secret("WATSONX_TOKEN"),
            watsonx_api_base: text("WATSONX_API_BASE"),
            watsonx_url: text("WATSONX_URL"),
            wx_url: text("WX_URL"),
            wml_url: text("WML_URL"),
            watsonx_project_id: text("WATSONX_PROJECT_ID"),
            wx_project_id: text("WX_PROJECT_ID"),
            project_id: text("PROJECT_ID"),
            watsonx_region: text("WATSONX_REGION"),
            wx_region: text("WX_REGION"),
            region: text("REGION"),
            watsonx_deployment_space_id: text("WATSONX_DEPLOYMENT_SPACE_ID"),
            watsonx_space_id: text("WATSONX_SPACE_ID"),
            wx_space_id: text("WX_SPACE_ID"),
            space_id: text("SPACE_ID"),
        }
    }

    #[must_use]
    pub fn api_base(&self) -> Option<&str> {
        first_set(&[
            self.watsonx_api_base.as_deref(),
            self.watsonx_url.as_deref(),
            self.wx_url.as_deref(),
            self.wml_url.as_deref(),
        ])
    }
}

pub(crate) fn first_set<'env>(values: &[Option<&'env str>]) -> Option<&'env str> {
    values
        .iter()
        .copied()
        .flatten()
        .find(|value| !value.is_empty())
}
