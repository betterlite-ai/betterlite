use thiserror::Error;

#[derive(Debug, Error)]
pub enum VolcEngineChatError {
    #[error("the Volcengine chat settings could not be read: {source}")]
    Settings {
        #[source]
        source: serde_json::Error,
    },
    #[error("the extra_body option is not an object, so a thinking option cannot go into it")]
    ExtraBodyNotObject,
}
