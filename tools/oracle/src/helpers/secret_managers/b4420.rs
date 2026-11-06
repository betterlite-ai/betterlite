use std::{
    collections::{BTreeMap, VecDeque},
    io,
    sync::{Arc, Mutex},
    time::Duration,
};

use betterlite_secret_managers::{
    google_secret_manager::{
        Dependencies, GoogleSecretManager, GoogleSecretManagerError, HttpResponse,
        NativeSecretReader, RequestHeaderSource, RequestHeaders, SecretTransport,
        settings::Options,
    },
    secret_contracts::SecretFuture,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    environment: BTreeMap<String, String>,
    refresh_interval: Option<u64>,
    always_read: Option<bool>,
    native: Option<NativeAnswer>,
    #[serde(default)]
    responses: Vec<ScriptedResponse>,
    #[serde(default)]
    steps: Vec<Step>,
}

#[derive(Clone, Debug, Deserialize)]
struct NativeAnswer {
    value: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct ScriptedResponse {
    status: u16,
    body: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Step {
    Read(String),
    Advance(u64),
}

#[derive(Clone, Serialize)]
struct Request {
    url: String,
    headers: BTreeMap<String, String>,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum StepResult {
    Value(Option<String>),
    Error(bool),
    Advanced(u64),
}

#[derive(Serialize)]
struct Output {
    initialized: bool,
    refresh_interval_seconds: u64,
    always_read: bool,
    steps: Vec<StepResult>,
    requests: Vec<Request>,
}

struct ScriptedTransport {
    answers: Mutex<VecDeque<ScriptedResponse>>,
    requests: Mutex<Vec<Request>>,
}

impl SecretTransport for ScriptedTransport {
    fn get<'request>(
        &'request self,
        url: &'request str,
        headers: &'request RequestHeaders,
    ) -> SecretFuture<'request, HttpResponse, io::Error> {
        let exposed = headers
            .iter()
            .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
            .collect();
        let answer = self
            .answers
            .lock()
            .ok()
            .and_then(|mut answers| answers.pop_front());
        if let Ok(mut requests) = self.requests.lock() {
            requests.push(Request {
                url: url.to_owned(),
                headers: exposed,
            });
        }
        Box::pin(async move {
            answer
                .map(|scripted| HttpResponse {
                    status: scripted.status,
                    body: scripted.body,
                })
                .ok_or_else(|| io::Error::other("no scripted response left"))
        })
    }
}

struct FixedHeaders;

impl RequestHeaderSource for FixedHeaders {
    fn sync_construct_request_headers(&self) -> Result<RequestHeaders, io::Error> {
        Ok(BTreeMap::from([
            (
                "Authorization".to_owned(),
                SecretString::from("Bearer oracle-token"),
            ),
            (
                "Content-Type".to_owned(),
                SecretString::from("application/json"),
            ),
        ]))
    }
}

impl NativeSecretReader for NativeAnswer {
    fn sync_read_secret(&self, _secret_name: &str) -> Result<Option<SecretString>, io::Error> {
        Ok(self.value.clone().map(SecretString::from))
    }
}

struct Session {
    manager: GoogleSecretManager,
    transport: Arc<ScriptedTransport>,
    clock_seconds: Arc<Mutex<u64>>,
}

fn open_session(input: &Input) -> Result<Session, GoogleSecretManagerError> {
    let transport = Arc::new(ScriptedTransport {
        answers: Mutex::new(input.responses.iter().cloned().collect()),
        requests: Mutex::new(Vec::new()),
    });
    let clock_seconds = Arc::new(Mutex::new(0_u64));
    let clock_reader = Arc::clone(&clock_seconds);
    let transport_dependency: Arc<dyn SecretTransport> =
        Arc::<ScriptedTransport>::clone(&transport);
    let native_reader = input.native.as_ref().map(|native| {
        let reader: Arc<dyn NativeSecretReader> = Arc::new(native.clone());
        reader
    });
    let manager = GoogleSecretManager::initialize(
        &input.environment,
        Options {
            refresh_interval: input.refresh_interval,
            always_read_secret_manager: input.always_read,
            request_timeout: None,
        },
        Dependencies {
            transport: transport_dependency,
            header_source: Arc::new(FixedHeaders),
            native_reader,
            clock: Arc::new(move || {
                Duration::from_secs(clock_reader.lock().map_or(0, |seconds| *seconds))
            }),
        },
    )?;
    Ok(Session {
        manager,
        transport,
        clock_seconds,
    })
}

fn play(runtime: &tokio::runtime::Runtime, session: &Session, steps: &[Step]) -> Vec<StepResult> {
    let mut results = Vec::new();
    for step in steps {
        match step {
            Step::Advance(seconds) => {
                if let Ok(mut now) = session.clock_seconds.lock() {
                    *now = now.saturating_add(*seconds);
                }
                results.push(StepResult::Advanced(*seconds));
            }
            Step::Read(name) => {
                let outcome = runtime.block_on(session.manager.get_secret(name));
                results.push(outcome.map_or(StepResult::Error(true), |secret| {
                    StepResult::Value(secret.map(|value| value.expose_secret().to_owned()))
                }));
            }
        }
    }
    results
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let Ok(session) = open_session(input) else {
            return Ok(serde_json::json!({"initialized": false}));
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(runtime_failure)?;
        let steps = play(&runtime, &session, &input.steps);
        let requests = session.transport.requests.lock().map(|sent| sent.clone());
        let settings = &session.manager.settings;
        serde_json::to_value(Output {
            initialized: true,
            refresh_interval_seconds: settings.refresh_interval.as_secs(),
            always_read: settings.always_read_secret_manager,
            steps,
            requests: requests.unwrap_or_default(),
        })
        .map_err(runtime_failure)
    }
}

fn runtime_failure<Source>(source: Source) -> Box<crate::helpers::HelperError>
where
    Source: std::fmt::Display,
{
    Box::new(crate::helpers::HelperError {
        class: "RuntimeError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: source.to_string(),
    })
}
