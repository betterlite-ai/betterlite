use secrecy::SecretString;
use serde::Deserialize;

pub(super) const DEFAULT_BASE_URL: &str = "https://gitlab.com/api/v4";
const DEFAULT_BRANCH: &str = "main";

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum ProjectId {
    Path(String),
    Number(i64),
}

impl ProjectId {
    pub(super) const fn is_blank(&self) -> bool {
        match self {
            Self::Path(path) => path.is_empty(),
            Self::Number(number) => *number == 0,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct GitLabConfig {
    pub project: Option<ProjectId>,
    pub access_token: Option<SecretString>,
    pub auth_method: Option<String>,
    pub branch: Option<String>,
    pub tag: Option<String>,
    pub base_url: Option<String>,
    pub prompts_path: Option<String>,
    pub folder: Option<String>,
}

impl GitLabConfig {
    // LiteLLM treats an empty branch or tag as absent, so a blank value never reaches the URL.
    pub(super) fn default_ref(&self) -> String {
        let branch = self
            .branch
            .as_deref()
            .filter(|name| !name.is_empty())
            .unwrap_or(DEFAULT_BRANCH);
        let tag = self.tag.as_deref().filter(|name| !name.is_empty());
        tag.unwrap_or(branch).to_owned()
    }
}

mod tests;
