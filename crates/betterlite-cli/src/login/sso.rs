use std::sync::Mutex;

use num_traits::ToPrimitive as _;
use secrecy::SecretString;
use url::form_urlencoded::Serializer;

use crate::credentials::contracts::{CliAuthResult, CliSsoStartData};
use crate::pkce_login::{Clock, Entropy, Http, WallClock};
use crate::sso_polling::session::{
    AuthenticationPoll, poll_for_authentication, start_cli_sso_flow,
};
use crate::sso_polling::transport::PollTransport;
use crate::token_records::record::CliTokenRecord;

use super::echo::echo_line;
use super::error::FlowError;
use super::finish::{LoginFinished, finish_login};
use super::ports::{FlowPorts, TerminalWriter};
use super::store::save_login_record;

pub const SOURCE_IDENTIFIER: &str = "litellm-cli";
pub const GENERATE_PATH: &str = "/sso/key/generate";

pub fn sso_login<H, C, E, W>(
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
    let start = start_cli_sso_flow(&flow.context.base_url, &*flow.http)?;
    let prefills = start.verification_uri_complete.is_some();
    let sso_url = generate_url(&flow.context.base_url, &start, prefills);
    write_browser_lines(stdout, &sso_url, &start, prefills)?;
    (flow.browser)(&sso_url);
    writeln!(stdout, "Waiting for authentication...")?;
    let Some(result) = poll_until_ready(flow, &mut *stdout, &start)? else {
        writeln!(stdout, "Authentication timed out. Please try again.")?;
        writeln!(
            stdout,
            "The proxy never reported the browser sign-in as finished. If you did complete it, check the proxy logs for /sso/callback errors and confirm SSO is configured on the proxy."
        )?;
        return Ok(());
    };
    let record = sso_record(&flow.context.base_url, &result, flow.wall_clock);
    let stored = save_login_record(flow, stdout, record)?;
    finish_login(
        LoginFinished {
            base_url: &flow.context.base_url,
            api_key: &result.api_key,
            config_claude,
            stored: &stored,
        },
        flow,
        stdout,
    )
}

fn generate_url(base_url: &str, start: &CliSsoStartData, prefills: bool) -> String {
    let mut query = Serializer::new(String::new());
    query.append_pair("source", SOURCE_IDENTIFIER);
    query.append_pair("key", &start.login_id);
    if prefills {
        query.append_pair("user_code", &start.user_code);
    }
    format!("{base_url}{GENERATE_PATH}?{}", query.finish())
}

fn write_browser_lines(
    stdout: &mut TerminalWriter,
    sso_url: &str,
    start: &CliSsoStartData,
    prefills: bool,
) -> Result<(), FlowError> {
    writeln!(stdout, "Opening browser to: {sso_url}")?;
    writeln!(
        stdout,
        "Please complete the SSO authentication in your browser..."
    )?;
    let suffix = if prefills {
        " (pre-filled in the browser, check it matches)"
    } else {
        ""
    };
    writeln!(stdout, "Verification code: {}{suffix}", start.user_code)?;
    writeln!(stdout, "Session ID: {}", start.login_id)?;
    Ok(())
}

fn poll_until_ready<H, C, E, W>(
    flow: &mut FlowPorts<'_, H, C, E, W>,
    stdout: &mut TerminalWriter,
    start: &CliSsoStartData,
) -> Result<Option<CliAuthResult>, FlowError>
where
    H: Http + PollTransport + Send + Sync + 'static,
{
    let sink = Mutex::new(&mut *stdout);
    let echo = |message: &str| echo_line(&sink, message);
    let mut poll = AuthenticationPoll {
        base_url: &flow.context.base_url,
        key_id: &start.login_id,
        poll_secret: &start.poll_secret,
        transport: &*flow.http,
        sleeper: flow.sleeper,
        lines: &mut *flow.selection,
        echo: &echo,
    };
    poll_for_authentication(&mut poll).map_err(FlowError::Polling)
}

fn sso_record(base_url: &str, result: &CliAuthResult, clock: &dyn WallClock) -> CliTokenRecord {
    CliTokenRecord {
        base_url: base_url.trim_end_matches('/').to_owned(),
        key: Some(result.api_key.clone()),
        user_id: result
            .user_id
            .clone()
            .unwrap_or_else(|| "cli-user".to_owned()),
        user_email: "unknown".to_owned(),
        user_role: "cli".to_owned(),
        auth_header_name: "Authorization".to_owned(),
        jwt_token: SecretString::default(),
        timestamp: clock.now_seconds().to_f64().unwrap_or_default(),
        expires_at: None,
        refresh_token: None,
        client_id: None,
        token_endpoint: None,
        revocation_endpoint: None,
        resource: None,
        team_id: None,
    }
}
