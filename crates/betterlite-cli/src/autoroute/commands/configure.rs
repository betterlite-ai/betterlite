use super::AutoroutePorts;
use super::error::CommandsError;
use crate::autoroute::wizard::{ConfigureRequest, run_configure_wizard};

pub fn configure(ports: &mut AutoroutePorts<'_>) -> Result<(), CommandsError> {
    let api_key = ports
        .context
        .api_key
        .clone()
        .ok_or(CommandsError::MissingApiKey)?;
    let base_url = ports.context.base_url.clone();
    let request = ConfigureRequest {
        home: ports.home,
        base_url,
        api_key,
        is_interactive: ports.stdin_is_terminal,
        client: ports.client,
        prompter: ports.prompter,
        files: ports.files,
    };
    let _written = run_configure_wizard(request)?;
    Ok(())
}
