use std::sync::Mutex;
use std::time::Duration;

use crate::credentials::login::{
    LOGIN_TIMEOUT_SECONDS, LoginRequest, pkce_token_record, run_pkce_login,
};
use crate::pkce_login::{Clock, Entropy, Http, WallClock};
use crate::sso_polling::transport::PollTransport;

use super::echo::echo_line;
use super::error::FlowError;
use super::finish::{LoginFinished, finish_login};
use super::ports::{FlowPorts, TerminalWriter};
use super::store::save_login_record;

pub fn pkce_login<H, C, E, W>(
    config_claude: bool,
    flow: &mut FlowPorts<'_, H, C, E, W>,
    stdout: &mut TerminalWriter,
) -> Result<(), FlowError>
where
    H: Http + PollTransport + Send + Sync + 'static,
    C: Clock,
    E: Entropy,
    W: WallClock + Send + Sync + 'static,
{
    let credential = {
        let sink = Mutex::new(&mut *stdout);
        let echo = |message: &str| echo_line(&sink, message);
        run_pkce_login(LoginRequest {
            base_url: &flow.context.base_url,
            http: &*flow.http,
            open_browser: flow.browser,
            echo: &echo,
            entropy: &mut *flow.entropy,
            clock: flow.clock,
            wall_clock: flow.wall_clock,
            timeout: Duration::from_secs(LOGIN_TIMEOUT_SECONDS),
        })
    }?;
    let record = pkce_token_record(&flow.context.base_url, &credential, flow.wall_clock);
    let stored = save_login_record(flow, stdout, record)?;
    finish_login(
        LoginFinished {
            base_url: &flow.context.base_url,
            api_key: &credential.access_token,
            config_claude,
            stored: &stored,
        },
        flow,
        stdout,
    )
}
