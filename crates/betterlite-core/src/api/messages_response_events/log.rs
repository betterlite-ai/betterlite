#[derive(Debug, Clone, serde::Serialize)]
pub struct StreamLog {
    pub level: &'static str,
    // The message as the wrapper logged it, kept so the iterator can pass it on and the oracle can compare it.
    pub message: String,
}
