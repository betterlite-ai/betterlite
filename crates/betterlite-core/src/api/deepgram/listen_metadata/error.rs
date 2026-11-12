use thiserror::Error;

#[derive(Debug, Error)]
pub enum ListenMetadataError {
    #[error("the upstream listen URL is not valid")]
    Target {
        #[source]
        source: url::ParseError,
    },
    #[error("the listen API base is not an absolute URL")]
    Base {
        #[source]
        source: url::ParseError,
    },
    #[error("the listen URL scheme cannot become {scheme}")]
    Scheme { scheme: String },
}
