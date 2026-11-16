use crate::commands::chat::{
    ChatArgs, FileConversationStore, StdTerminal as ChatTerminal, run_chat_command,
};

use super::super::Failure;
use super::super::session::Session;

#[cfg_attr(test, mutants::skip)]
pub(crate) fn run(session: &mut Session<'_>, args: &ChatArgs) -> Result<(), Failure> {
    Ok(run_chat_command(
        args,
        &session.context,
        &session.client,
        &mut ChatTerminal,
        &mut FileConversationStore,
    )?)
}
