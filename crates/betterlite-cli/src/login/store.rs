use std::sync::{Arc, Mutex};

use tokio::runtime::Builder;

use crate::credential_lifecycle::outcome::SecretSave;
use crate::credentials::revocation::{ReplaceStoredTokenRequest, replace_stored_token};
use crate::pkce_login::Http;
use crate::token_records::record::CliTokenRecord;

use super::echo::echo_line;
use super::error::FlowError;
use super::ports::{FlowPorts, TerminalWriter};

pub fn save_login_record<H, C, E, W>(
    flow: &mut FlowPorts<'_, H, C, E, W>,
    stdout: &mut TerminalWriter,
    record: CliTokenRecord,
) -> Result<SecretSave, FlowError>
where
    H: Http + Send + Sync + 'static,
{
    let sink = Mutex::new(&mut *stdout);
    let echo = |message: &str| echo_line(&sink, message);
    let runtime = Builder::new_current_thread()
        .build()
        .map_err(FlowError::Runtime)?;
    let saved = runtime.block_on(replace_stored_token(ReplaceStoredTokenRequest {
        record,
        files: flow.files,
        vault: &mut *flow.vault,
        http: Arc::clone(&flow.http),
        echo: &echo,
    }));
    Ok(saved)
}
