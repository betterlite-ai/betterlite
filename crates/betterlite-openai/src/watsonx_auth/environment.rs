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
