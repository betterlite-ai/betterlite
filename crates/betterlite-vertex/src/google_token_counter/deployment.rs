use std::time::Duration;

use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use secrecy::SecretString;

use crate::credential_sources::getters::VertexParams;
use crate::studio_identity::AuthValue;

// the deployment's litellm_params a count call reads; a call takes them by value and never writes them back
#[derive(Clone, Copy, Debug)]
pub struct DeploymentParams<'call> {
    pub model: Option<&'call str>,
    pub api_key: Option<&'call SecretString>,
    pub gemini_api_key: Option<&'call AuthValue>,
    pub api_base: Option<&'call str>,
    pub timeout: Option<Duration>,
    pub vertex: VertexParams<'call>,
    pub vertex_count_tokens_location: Option<&'call str>,
}

impl<'call> DeploymentParams<'call> {
    // LiteLLM reads each alias as `vertex_project or vertex_ai_project`, so an empty spelling falls through
    #[must_use]
    pub fn explicit_project(&self) -> Option<&'call str> {
        self.vertex
            .project
            .filter(|project| !project.is_empty())
            .or_else(|| self.vertex.ai_project.filter(|project| !project.is_empty()))
    }

    // LiteLLM prefers vertex_count_tokens_location over either location spelling for partner counts
    #[must_use]
    pub fn explicit_location(&self) -> Option<&'call str> {
        self.vertex_count_tokens_location
            .filter(|location| !location.is_empty())
            .or_else(|| self.vertex.explicit_vertex_ai_location())
    }

    #[must_use]
    pub fn explicit_credentials(&self) -> Option<&'call ConfiguredCredentials> {
        self.vertex
            .credentials
            .filter(|credentials| credentials.as_text().is_none_or(|text| !text.is_empty()))
            .or_else(|| {
                self.vertex
                    .ai_credentials
                    .filter(|credentials| credentials.as_text().is_none_or(|text| !text.is_empty()))
            })
    }
}
