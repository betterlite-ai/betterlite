use betterlite_config::tls::Verification;
use serde::Serialize;

// what LiteLLM hands websockets.connect as ssl=: None, True for no verification, or the shared context
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RealtimeSsl {
    None,
    Insecure,
    Shared,
}

#[must_use]
pub fn ssl_config(url: &str, verification: &Verification) -> RealtimeSsl {
    if url.starts_with("ws://") {
        return RealtimeSsl::None;
    }
    match verification {
        Verification::Disabled => RealtimeSsl::Insecure,
        Verification::Enabled | Verification::CaFile(_) => RealtimeSsl::Shared,
    }
}
