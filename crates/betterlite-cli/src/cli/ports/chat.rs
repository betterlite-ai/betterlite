use crate::commands::chat::conversation::FileConversationStore;
use crate::commands::chat::{ChatArgs, StdTerminal as ChatTerminal, run_chat_command};

use super::super::Failure;
use super::super::session::Session;

#[cfg_attr(test, mutants::skip)]
pub(crate) fn run(session: &Session<'_>, args: &ChatArgs) -> Result<(), Failure> {
    Ok(run_chat_command(
        args,
        &session.context,
        &session.client,
        &mut ChatTerminal,
        &mut FileConversationStore,
    )?)
}
