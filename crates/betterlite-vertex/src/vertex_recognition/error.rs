use http::header::InvalidHeaderValue;
use thiserror::Error;

use crate::gemini_response::decode::MAX_RESPONSE_BYTES;
use crate::model_urls::error::VertexUrlError;

#[derive(Debug, Error)]
pub enum VertexRecognitionError {
    #[error("the Vertex location is not valid: {source}")]
    Location {
        #[source]
        source: VertexUrlError,
    },
    #[error("vertex_project is required to build a Speech-to-Text URL")]
    ProjectRequired,
    #[error("the vertex_project '{project}' cannot be used in a URL")]
    Project { project: String },
    #[error("Google Speech-to-Text supports only the json and text response formats, not {format}")]
    ResponseFormat { format: String },
    #[error("the Google Speech-to-Text reply is not JSON")]
    NotJson {
        status: u16,
        #[source]
        source: serde_json::Error,
    },
    #[error("the Google Speech-to-Text reply does not match the recognize response at {path}")]
    Shape {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error(
        "the Google Speech-to-Text reply is {size} bytes, over the {MAX_RESPONSE_BYTES} byte limit"
    )]
    TooLarge { size: usize },
    #[error("the authorization header cannot carry the access token")]
    Authorization {
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the x-goog-user-project header cannot carry the vertex_project")]
    UserProject {
        #[source]
        source: InvalidHeaderValue,
    },
}

impl VertexRecognitionError {
    // the location, project and response_format failures are VertexAIError(400); the rest are not provider replies
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Location { .. }
            | Self::ProjectRequired
            | Self::Project { .. }
            | Self::ResponseFormat { .. } => Some(400),
            Self::NotJson { status, .. } => Some(*status),
            Self::Shape { .. }
            | Self::TooLarge { .. }
            | Self::Authorization { .. }
            | Self::UserProject { .. } => None,
        }
    }
}
