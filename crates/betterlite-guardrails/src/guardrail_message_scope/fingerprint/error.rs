#[derive(Debug, thiserror::Error)]
pub enum FingerprintError {
    #[error("a stream item that is raw bytes has no JSON form to fingerprint")]
    Bytes,
    #[error("the stream item could not be written as JSON")]
    Serialize {
        #[source]
        source: serde_json::Error,
    },
}
