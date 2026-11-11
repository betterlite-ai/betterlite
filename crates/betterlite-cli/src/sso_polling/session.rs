use secrecy::SecretString;
use serde::Deserialize;

use crate::credentials::contracts::{CliAuthResult, CliPollData, CliSsoStartData, CliTeam};
use crate::credentials::notice::MessageSink;
use crate::pkce_login::HttpResponse;

use super::error::PollingError;
use super::polling::{
    PollNotice, PollOptions, ReadyPoll, Sleeper, poll_for_ready_data, response_error_detail,
};
use super::selection::prompt_team_id;
use super::teams::normalize_teams;
use super::terminal::SelectionInput;
use super::transport::PollTransport;
use super::wire;
use super::{
    DEFAULT_LOG_EVERY, DEFAULT_POLL_INTERVAL_SECONDS, DEFAULT_REQUEST_TIMEOUT_SECS,
    DEFAULT_TOTAL_TIMEOUT_SECONDS, POLL_SECRET_HEADER, SSO_POLL_PATH, SSO_START_PATH,
    SSO_START_TIMEOUT_SECONDS,
};

const WAITING_AUTHENTICATION_MESSAGE: &str = "Still waiting for authentication...";
const WAITING_TEAM_MESSAGE: &str = "Still waiting for team authentication...";
const TEAM_INCOMPLETE_MESSAGE: &str = "Waiting for team authentication to complete...";
const NO_TEAMS_WARNING: &str = "Warning: No teams available for selection.";
const TEAM_SELECTION_FAILED: &str = "Team selection cancelled or JWT generation failed.";
const NO_TEAMS_MESSAGE: &str =
    "No teams found. You can create or join teams using the web interface.";
const NO_TEAM_SELECTED_MESSAGE: &str = "No team selected.";
const SELECT_TEAM_INTRO: &str = "Select a team for your CLI session...";
const TEAM_SEPARATOR: &str = "\n============================================================";

#[derive(Deserialize)]
struct StartWire {
    #[serde(default)]
    login_id: Option<String>,
    #[serde(default)]
    poll_secret: Option<SecretString>,
    #[serde(default)]
    user_code: Option<String>,
    #[serde(default)]
    verification_uri_complete: Option<String>,
}

pub fn start_cli_sso_flow(
    base_url: &str,
    transport: &dyn PollTransport,
) -> Result<CliSsoStartData, PollingError> {
    let start_url = format!("{base_url}{SSO_START_PATH}");
    let response = transport
        .post(&start_url, SSO_START_TIMEOUT_SECONDS)
        .map_err(|source| PollingError::StartUnreachable {
            url: start_url.clone(),
            source,
        })?;
    match response.status {
        200 => parse_start_body(&start_url, &response),
        404 | 405 => Err(PollingError::StartUnsupported {
            status: response.status,
            url: start_url,
        }),
        _ => Err(start_failure(&start_url, &response)),
    }
}

fn parse_start_body(url: &str, response: &HttpResponse) -> Result<CliSsoStartData, PollingError> {
    let body: StartWire = match wire::decode_object(&response.body) {
        Ok(body) => body,
        Err(source) if source.is_data_error() => {
            return Err(PollingError::StartBody {
                url: url.to_owned(),
                source,
            });
        }
        Err(_) => {
            return Err(PollingError::StartNotJson {
                url: url.to_owned(),
                content_type: content_type(response),
            });
        }
    };
    let missing = missing_fields(&body);
    let start = body.login_id.zip(body.poll_secret).zip(body.user_code);
    let Some(((login_id, poll_secret), user_code)) = start else {
        return Err(PollingError::StartMissingFields {
            url: url.to_owned(),
            fields: missing.join(", "),
        });
    };
    Ok(CliSsoStartData {
        login_id,
        poll_secret,
        user_code,
        verification_uri_complete: body.verification_uri_complete,
    })
}

fn content_type(response: &HttpResponse) -> String {
    response
        .headers
        .iter()
        .find(|(name, _value)| name.eq_ignore_ascii_case("content-type"))
        .map_or_else(|| String::from("unknown"), |(_name, value)| value.clone())
}

fn missing_fields(body: &StartWire) -> Vec<&'static str> {
    let mut fields = Vec::new();
    if body.login_id.is_none() {
        fields.push("login_id");
    }
    if body.poll_secret.is_none() {
        fields.push("poll_secret");
    }
    if body.user_code.is_none() {
        fields.push("user_code");
    }
    fields
}

fn start_failure(url: &str, response: &HttpResponse) -> PollingError {
    response_error_detail(response).map_or_else(
        || PollingError::StartFailed {
            status: response.status,
            url: url.to_owned(),
        },
        |detail| PollingError::StartFailedWithDetail {
            status: response.status,
            url: url.to_owned(),
            detail,
        },
    )
}

pub struct AuthenticationPoll<'poll> {
    pub base_url: &'poll str,
    pub key_id: &'poll str,
    pub poll_secret: &'poll SecretString,
    pub transport: &'poll dyn PollTransport,
    pub sleeper: &'poll dyn Sleeper,
    pub lines: &'poll mut dyn SelectionInput,
    pub echo: MessageSink<'poll>,
}

impl AuthenticationPoll<'_> {
    fn poll_ready(&self, options: &PollOptions) -> Result<Option<CliPollData>, PollingError> {
        let url = format!("{}{SSO_POLL_PATH}{}", self.base_url, self.key_id);
        let headers = [(String::from(POLL_SECRET_HEADER), self.poll_secret.clone())];
        poll_for_ready_data(&ReadyPoll {
            url: &url,
            headers: &headers,
            options,
            transport: self.transport,
            sleeper: self.sleeper,
            echo: self.echo,
        })
    }

    fn select_team(&mut self, data: CliPollData) -> Result<Option<CliAuthResult>, PollingError> {
        let teams = normalize_teams(&data.teams, data.team_details.as_deref())?;
        if teams.is_empty() {
            (self.echo)(NO_TEAMS_WARNING);
            return Ok(None);
        }
        let jwt = handle_poll_team_selection(self, &teams)?;
        let Some(api_key) = jwt else {
            (self.echo)(TEAM_SELECTION_FAILED);
            return Ok(None);
        };
        Ok(Some(CliAuthResult {
            api_key,
            user_id: data.user_id,
            teams: data.teams,
            team_id: None,
        }))
    }
}

pub fn poll_for_authentication(
    poll: &mut AuthenticationPoll<'_>,
) -> Result<Option<CliAuthResult>, PollingError> {
    let options = PollOptions {
        total_timeout_seconds: DEFAULT_TOTAL_TIMEOUT_SECONDS,
        poll_interval_seconds: DEFAULT_POLL_INTERVAL_SECONDS,
        request_timeout_seconds: DEFAULT_REQUEST_TIMEOUT_SECS,
        pending: Some(PollNotice {
            message: String::from(WAITING_AUTHENTICATION_MESSAGE),
            every: DEFAULT_LOG_EVERY,
        }),
        other_status: None,
        http_error_log_every: DEFAULT_LOG_EVERY,
        connection_error_log_every: DEFAULT_LOG_EVERY,
    };
    let Some(data) = poll.poll_ready(&options)? else {
        return Ok(None);
    };
    if data.requires_team_selection {
        return poll.select_team(data);
    }
    Ok(direct_auth_result(data, poll.echo))
}

fn direct_auth_result(data: CliPollData, echo: MessageSink<'_>) -> Option<CliAuthResult> {
    if let Some(assigned) = data.team_id.as_deref()
        && data.teams.len() == 1
    {
        echo(&format!("\nAutomatically assigned to team: {assigned}"));
    }
    let api_key = data.key?;
    Some(CliAuthResult {
        api_key,
        user_id: data.user_id,
        teams: data.teams,
        team_id: data.team_id,
    })
}

pub fn handle_poll_team_selection(
    poll: &mut AuthenticationPoll<'_>,
    teams: &[CliTeam],
) -> Result<Option<SecretString>, PollingError> {
    if teams.is_empty() {
        (poll.echo)(NO_TEAMS_MESSAGE);
        return Ok(None);
    }
    (poll.echo)(TEAM_SEPARATOR);
    (poll.echo)(SELECT_TEAM_INTRO);
    let Some(team_id) = prompt_team_id(teams, &mut *poll.lines, poll.echo) else {
        (poll.echo)(NO_TEAM_SELECTED_MESSAGE);
        return Ok(None);
    };
    (poll.echo)(&format!("\nGenerating JWT for team: {team_id}"));
    let jwt = poll_team_jwt(poll, &team_id)?;
    if jwt.is_some() {
        (poll.echo)(&format!("Successfully generated JWT for team: {team_id}"));
    }
    Ok(jwt)
}

fn poll_team_jwt(
    poll: &AuthenticationPoll<'_>,
    team_id: &str,
) -> Result<Option<SecretString>, PollingError> {
    let options = PollOptions {
        total_timeout_seconds: DEFAULT_TOTAL_TIMEOUT_SECONDS,
        poll_interval_seconds: DEFAULT_POLL_INTERVAL_SECONDS,
        request_timeout_seconds: DEFAULT_REQUEST_TIMEOUT_SECS,
        pending: Some(PollNotice {
            message: String::from(WAITING_TEAM_MESSAGE),
            every: DEFAULT_LOG_EVERY,
        }),
        other_status: Some(PollNotice {
            message: String::from(TEAM_INCOMPLETE_MESSAGE),
            every: DEFAULT_LOG_EVERY,
        }),
        http_error_log_every: DEFAULT_LOG_EVERY,
        connection_error_log_every: DEFAULT_LOG_EVERY,
    };
    let url = format!(
        "{}{SSO_POLL_PATH}{}?team_id={team_id}",
        poll.base_url, poll.key_id
    );
    let headers = [(String::from(POLL_SECRET_HEADER), poll.poll_secret.clone())];
    let data = poll_for_ready_data(&ReadyPoll {
        url: &url,
        headers: &headers,
        options: &options,
        transport: poll.transport,
        sleeper: poll.sleeper,
        echo: poll.echo,
    })?;
    let Some(ready) = data else {
        return Ok(None);
    };
    Ok(ready.key)
}
