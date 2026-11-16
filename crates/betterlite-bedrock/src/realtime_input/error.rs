use thiserror::Error;

#[derive(Debug, Error)]
pub enum InputError {
    #[error("the realtime client message is not valid JSON")]
    InvalidJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("the realtime client message has the wrong shape")]
    Malformed {
        #[source]
        source: serde_json::Error,
    },
}
