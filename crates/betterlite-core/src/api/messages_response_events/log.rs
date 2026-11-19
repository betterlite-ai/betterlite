// One diagnostic the transition emitted, kept so the iterator can pass it on and the oracle can compare it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StreamLog {
    pub level: &'static str,
    pub message: String,
}
