use secrecy::SecretString;

use crate::agent_launch::Terminal;
use crate::agent_run::pipeline::{RunAgentRequest, run_agent};

use super::AgentPorts;
use super::error::AgentDispatchError;

pub fn launch_agent<H, E, W>(
    api_key: &SecretString,
    command: &[String],
    skip_verify: bool,
    terminal: Option<&dyn Terminal>,
    ports: &mut AgentPorts<'_, H, E, W>,
) -> Result<i32, AgentDispatchError> {
    run_agent(RunAgentRequest {
        base_url: &ports.context.base_url,
        api_key,
        command,
        skip_verify,
        base_env: ports.base_env,
        home: ports.home,
        resolver: ports.resolver,
        probe: ports.probe,
        sync: ports.sync,
        preparers: ports.preparers,
        launcher: ports.launcher,
        platform: ports.platform,
        terminal,
        messages: &mut *ports.messages,
    })
    .map_err(AgentDispatchError::Agent)
}
