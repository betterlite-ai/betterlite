use std::collections::VecDeque;
use std::io::Error as IoError;
use std::iter::repeat_with;
use std::sync::Mutex;
use std::time::Duration;

use console::Key;
use secrecy::{ExposeSecret as _, SecretString};

use crate::credentials::contracts::{CliAuthResult, CliPollData, CliTeam};
use crate::pkce_login::HttpResponse;
use crate::sso_polling::error::PollingError;
use crate::sso_polling::polling::{
    PollNotice, PollOptions, ReadyPoll, Sleeper, poll_for_ready_data,
};
use crate::sso_polling::session::{
    AuthenticationPoll, TeamSelectionPoll, handle_poll_team_selection, poll_for_authentication,
};
use crate::sso_polling::terminal::SelectionInput;
use crate::sso_polling::transport::PollTransport;

pub(super) const SERVER: &str = "https://gateway.example";
pub(super) const POLL_URL: &str = "https://gateway.example/poll";
pub(super) const POLL_SECRET: &str = "poll-secret";
pub(super) const LOGIN_ID: &str = "login-1";

pub(super) fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

pub(super) fn response(status: u16, body: &str) -> HttpResponse {
    HttpResponse {
        status,
        headers: Vec::new(),
        body: body.as_bytes().to_vec(),
    }
}

pub(super) fn typed_response(status: u16, content_type: &str, body: &str) -> HttpResponse {
    HttpResponse {
        status,
        headers: vec![(String::from("content-type"), content_type.to_owned())],
        body: body.as_bytes().to_vec(),
    }
}

pub(super) fn team(team_id: &str, alias: Option<&str>) -> CliTeam {
    CliTeam {
        team_id: Some(team_id.to_owned()),
        team_alias: alias.map(str::to_owned),
        models: Vec::new(),
        max_budget: None,
    }
}

pub(super) fn wait_options() -> PollOptions {
    PollOptions {
        total_timeout_seconds: 100,
        poll_interval_seconds: 2,
        request_timeout_seconds: 10,
        pending: Some(PollNotice {
            message: String::from("pending message"),
            every: 10,
        }),
        other_status: Some(PollNotice {
            message: String::from("other status message"),
            every: 10,
        }),
        http_error_log_every: 10,
        connection_error_log_every: 10,
    }
}

#[derive(Default)]
pub(super) struct FakeInput {
    pub(super) interactive: bool,
    pub(super) keys: VecDeque<Option<Key>>,
    pub(super) lines: VecDeque<Option<String>>,
}

pub(super) fn interactive_input(keys: Vec<Key>) -> FakeInput {
    FakeInput {
        interactive: true,
        keys: keys.into_iter().map(Some).collect(),
        lines: VecDeque::new(),
    }
}

pub(super) fn terminal_input(keys: Vec<Key>, lines: Vec<&str>) -> FakeInput {
    FakeInput {
        interactive: true,
        keys: keys.into_iter().map(Some).collect(),
        lines: lines
            .into_iter()
            .map(|line| Some(line.to_owned()))
            .collect(),
    }
}

pub(super) fn piped_input(lines: Vec<&str>) -> FakeInput {
    FakeInput {
        interactive: false,
        keys: VecDeque::new(),
        lines: lines
            .into_iter()
            .map(|line| Some(line.to_owned()))
            .collect(),
    }
}

impl SelectionInput for FakeInput {
    fn is_interactive(&self) -> bool {
        self.interactive
    }

    fn read_key(&mut self) -> Option<Key> {
        self.keys.pop_front().flatten()
    }

    fn read_line(&mut self) -> Option<String> {
        self.lines.pop_front().flatten()
    }
}

#[derive(Default)]
pub(super) struct Echo {
    messages: Mutex<Vec<String>>,
}

impl Echo {
    pub(super) fn sink(&self) -> impl Fn(&str) + Send + Sync + '_ {
        move |message| {
            self.messages
                .lock()
                .expect("the echo log")
                .push(message.to_owned());
        }
    }

    pub(super) fn recorded(&self) -> Vec<String> {
        self.messages.lock().expect("the echo log").clone()
    }
}

#[derive(Default)]
pub(super) struct RecordingSleeper {
    intervals: Mutex<Vec<Duration>>,
}

impl RecordingSleeper {
    pub(super) fn intervals(&self) -> Vec<Duration> {
        self.intervals.lock().expect("the sleep log").clone()
    }
}

impl Sleeper for RecordingSleeper {
    fn sleep(&self, interval: Duration) {
        self.intervals.lock().expect("the sleep log").push(interval);
    }
}

#[derive(Clone)]
pub(super) enum Reply {
    Response(HttpResponse),
    Transport,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum PollCall {
    Get {
        url: String,
        headers: Vec<(String, String)>,
        timeout: u64,
    },
    Post {
        url: String,
        timeout: u64,
    },
}

#[derive(Default)]
pub(super) struct ScriptedTransport {
    replies: Mutex<VecDeque<Reply>>,
    calls: Mutex<Vec<PollCall>>,
}

pub(super) fn scripted_transport(replies: Vec<Reply>) -> ScriptedTransport {
    ScriptedTransport {
        replies: Mutex::new(VecDeque::from(replies)),
        calls: Mutex::new(Vec::new()),
    }
}

pub(super) fn repeated(reply: &Reply, count: usize) -> Vec<Reply> {
    repeat_with(|| reply.clone()).take(count).collect()
}

pub(super) fn pending() -> Reply {
    Reply::Response(response(200, r#"{"status":"pending"}"#))
}

impl ScriptedTransport {
    pub(super) fn calls(&self) -> Vec<PollCall> {
        self.calls.lock().expect("the call log").clone()
    }

    fn answer(&self, call: PollCall) -> Result<HttpResponse, IoError> {
        self.calls
            .lock()
            .map_err(|_poisoned| IoError::other("the call log was poisoned"))?
            .push(call);
        let reply = self
            .replies
            .lock()
            .map_err(|_poisoned| IoError::other("the reply queue was poisoned"))?
            .pop_front();
        match reply {
            Some(Reply::Response(response)) => Ok(response),
            Some(Reply::Transport) => Err(IoError::other("connection refused")),
            None => Err(IoError::other("no scripted reply left")),
        }
    }
}

impl PollTransport for ScriptedTransport {
    fn get(
        &self,
        url: &str,
        headers: &[(String, SecretString)],
        timeout_seconds: u64,
    ) -> Result<HttpResponse, IoError> {
        let recorded_headers = headers
            .iter()
            .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
            .collect();
        self.answer(PollCall::Get {
            url: url.to_owned(),
            headers: recorded_headers,
            timeout: timeout_seconds,
        })
    }

    fn post(&self, url: &str, timeout_seconds: u64) -> Result<HttpResponse, IoError> {
        self.answer(PollCall::Post {
            url: url.to_owned(),
            timeout: timeout_seconds,
        })
    }
}

pub(super) fn poll(
    transport: &ScriptedTransport,
    options: &PollOptions,
    sleeper: &RecordingSleeper,
    echo: &Echo,
) -> Result<Option<CliPollData>, PollingError> {
    let sink = echo.sink();
    poll_for_ready_data(&ReadyPoll {
        url: POLL_URL,
        headers: &[],
        options,
        transport,
        sleeper,
        echo: &sink,
    })
}

pub(super) fn authenticate(
    transport: &ScriptedTransport,
    sleeper: &RecordingSleeper,
    input: &mut FakeInput,
    echo: &Echo,
) -> Result<Option<CliAuthResult>, PollingError> {
    let sink = echo.sink();
    let poll_secret = secret(POLL_SECRET);
    poll_for_authentication(&mut AuthenticationPoll {
        base_url: SERVER,
        key_id: LOGIN_ID,
        poll_secret: &poll_secret,
        transport,
        sleeper,
        lines: input,
        echo: &sink,
    })
}

pub(super) fn select_team(
    transport: &ScriptedTransport,
    sleeper: &RecordingSleeper,
    input: &mut FakeInput,
    teams: &[CliTeam],
    echo: &Echo,
) -> Result<Option<SecretString>, PollingError> {
    let sink = echo.sink();
    let poll_secret = secret(POLL_SECRET);
    handle_poll_team_selection(&mut TeamSelectionPoll {
        base_url: SERVER,
        key_id: LOGIN_ID,
        poll_secret: &poll_secret,
        transport,
        sleeper,
        lines: input,
        teams,
        echo: &sink,
    })
}
