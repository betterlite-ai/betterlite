use thiserror::Error;

#[derive(Debug, Clone, Error)]
pub enum SigningConfigDefect {
    #[error("mcp_session_token_signing is malformed")]
    Malformed,
    #[error("mcp_session_token_signing.private_key reference did not resolve")]
    UnresolvedPrivateKey,
    #[error(
        "mcp_session_token_signing.previous_public_keys reference did not resolve for kid(s): {}",
        kids.join(", ")
    )]
    UnresolvedPreviousKeys { kids: Vec<String> },
}
