use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Surface {
    Sdk,
    Proxy,
}

impl Surface {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sdk => "sdk",
            Self::Proxy => "proxy",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Deployment {
    #[serde(rename = "pip / Python SDK")]
    PythonSdk,
    Docker,
}

impl Deployment {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PythonSdk => "pip / Python SDK",
            Self::Docker => "Docker",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentReport {
    pub surface: Surface,
    pub litellm_version: String,
    pub python_version: String,
    pub deployment: Option<Deployment>,
    pub config_lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BugReport {
    pub environment: EnvironmentReport,
    pub exception_type: String,
    pub litellm_frames: Vec<String>,
    pub call_type: Option<String>,
    pub custom_llm_provider: Option<String>,
    pub stream: Option<bool>,
}
