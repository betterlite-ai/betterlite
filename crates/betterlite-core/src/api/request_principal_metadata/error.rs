use thiserror::Error;

#[derive(Debug, Error)]
pub enum RequestPrincipalMetadataError {
    #[error("the principal's request metadata could not be encoded")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error(
        "invalid callback variable {name} in the team's default settings; it must name a dynamic callback parameter"
    )]
    CallbackVariable { name: String },
}
