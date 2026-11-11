use std::io;

use thiserror::Error;

use crate::claude_settings::receipts::error::ReceiptsError;

#[derive(Debug, Error)]
pub enum SessionError {
    #[error("the async runtime could not start: {source}")]
    Runtime {
        #[source]
        source: io::Error,
    },
    #[error(
        "the proxy could not record the revocation ({reason}), so nothing was cleared; run `lite logout` again shortly"
    )]
    RevocationUnavailable { reason: String },
    #[error("not signed in; run `lite login` first")]
    NotAuthenticated,
    #[error(
        "the stored credential was issued for another server; run `lite login` for this one first"
    )]
    NotAuthenticatedForServer,
    #[error("the stored token has expired; run `lite login` again")]
    TokenExpired,
    #[error("{notice}")]
    KeychainUnreadable { notice: String },
    #[error("the stored key could not be used; run `{login_command}` again")]
    UnusableKey { login_command: &'static str },
    #[error("cannot find your home directory, so Claude Code cannot be configured")]
    NoHome,
    #[error("the login worked, but Claude Code could not be configured: {source}")]
    ConfigureClaude {
        #[from]
        #[source]
        source: ReceiptsError,
    },
}
