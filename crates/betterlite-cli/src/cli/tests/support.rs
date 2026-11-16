use std::collections::BTreeMap;
use std::io::{self, ErrorKind, Read as _, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use betterlite_config::loader::EnvironmentLookup;
use secrecy::SecretString;

use crate::agent_launch::EnvVars;
use crate::cli::session::{CliClock, Session, SessionParts};
use crate::cli::streams::{Channel, Streams};
use crate::cli::{Cli, run_with};
use crate::cli_context::CliContext;
use crate::commands::Client;
use crate::keyring::{
    Erase, Read, SecretErase, SecretRead, SecretVault, SecretWrite, Write as VaultWrite,
};

#[derive(Default)]
pub(crate) struct FakeEnvironment {
    pub(crate) variables: BTreeMap<String, String>,
    pub(crate) home: Option<PathBuf>,
}

impl FakeEnvironment {
    pub(crate) fn with_home(home: PathBuf) -> Self {
        Self {
            variables: BTreeMap::new(),
            home: Some(home),
        }
    }

    pub(crate) fn with_variable(mut self, name: &str, value: &str) -> Self {
        let _inserted = self.variables.insert(name.to_owned(), value.to_owned());
        self
    }
}

impl EnvironmentLookup for FakeEnvironment {
    fn lookup(&self, name: &str) -> Option<String> {
        self.variables.get(name).cloned()
    }

    fn system_home(&self) -> Option<PathBuf> {
        self.home.clone()
    }
}

#[derive(Default)]
pub(crate) struct FakeVault;

impl SecretVault for FakeVault {
    fn read(&self) -> SecretRead {
        Ok(Read::SecretMissing)
    }

    fn write(&mut self, _blob: &SecretString) -> SecretWrite {
        Ok(VaultWrite::SecretStored)
    }

    fn erase(&mut self) -> SecretErase {
        Ok(Erase::SecretErased)
    }
}

#[derive(Clone, Default)]
pub(crate) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(crate) fn text(&self) -> String {
        let bytes = self.0.lock().map(|bytes| bytes.clone()).unwrap_or_default();
        String::from_utf8(bytes).unwrap_or_default()
    }
}

pub(crate) fn channel(sink: Box<dyn Write + Send>) -> Channel {
    Channel(Arc::new(Mutex::new(sink)))
}

pub(crate) const ACCEPT_DEADLINE: Duration = Duration::from_secs(5);
const POLL_PAUSE: Duration = Duration::from_millis(5);

pub(crate) struct Answer {
    pub(crate) status: u16,
    pub(crate) reason: &'static str,
    pub(crate) body: &'static str,
}

pub(crate) struct Served {
    pub(crate) base_url: String,
    request: Arc<Mutex<Vec<u8>>>,
}

impl Served {
    pub(crate) fn request(&self) -> String {
        let bytes = self
            .request
            .lock()
            .map(|recorded| recorded.clone())
            .unwrap_or_default();
        String::from_utf8(bytes).unwrap_or_default()
    }
}

pub(crate) fn serve<'scope>(scope: &'scope thread::Scope<'scope, '_>, answer: Answer) -> Served {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let request = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&request);
    scope.spawn(move || accept_once(&listener, &answer, &recorded));
    Served {
        base_url: format!("http://{address}"),
        request,
    }
}

fn accept_once(listener: &TcpListener, answer: &Answer, request: &Mutex<Vec<u8>>) {
    let deadline = Instant::now().checked_add(ACCEPT_DEADLINE).unwrap();
    while Instant::now() < deadline {
        match listener.accept() {
            Ok((stream, _peer)) => return respond(stream, answer, request),
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                thread::park_timeout(POLL_PAUSE);
            }
            Err(_error) => return,
        }
    }
}

fn respond(mut stream: TcpStream, answer: &Answer, request: &Mutex<Vec<u8>>) {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut buffer = [0_u8; 4096];
    let read = stream.read(&mut buffer).unwrap_or(0);
    if let Ok(mut recorded) = request.lock() {
        recorded.extend_from_slice(&buffer[..read]);
    }
    let head = format!(
        "HTTP/1.1 {} {}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        answer.status,
        answer.reason,
        answer.body.len()
    );
    let _wrote_head = stream.write_all(head.as_bytes());
    let _wrote_body = stream.write_all(answer.body.as_bytes());
    let _flushed = stream.flush();
    let _shut = stream.shutdown(Shutdown::Write);
}

pub(crate) fn unused_base_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    format!("http://{address}")
}

impl Write for Captured {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        match self.0.lock() {
            Ok(mut bytes) => bytes.extend_from_slice(buffer),
            Err(_poisoned) => return Err(io::Error::other("the capture lock was poisoned")),
        }
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) struct Harness {
    pub(crate) environment: FakeEnvironment,
    pub(crate) vault: FakeVault,
    pub(crate) streams: Streams,
    pub(crate) out: Captured,
    pub(crate) err: Captured,
}

impl Harness {
    pub(crate) fn new(environment: FakeEnvironment) -> Self {
        let out = Captured::default();
        let err = Captured::default();
        Self {
            environment,
            vault: FakeVault,
            streams: Streams::new(Box::new(out.clone()), Box::new(err.clone())),
            out,
            err,
        }
    }

    pub(crate) fn run(&mut self, arguments: &[&str]) -> u8 {
        let arguments: Vec<String> = arguments
            .iter()
            .map(|argument| (*argument).to_owned())
            .collect();
        let cli = Cli {
            environment: &self.environment,
            vault: &mut self.vault,
            streams: self.streams.clone(),
            base_env: EnvVars::new(),
            stdin_is_terminal: false,
        };
        run_with(&arguments, cli)
    }

    pub(crate) fn stdout(&self) -> String {
        self.out.text()
    }

    pub(crate) fn stderr(&self) -> String {
        self.err.text()
    }
}

pub(crate) fn proxy_context(base_url: &str) -> CliContext {
    CliContext {
        base_url: base_url.to_owned(),
        base_url_explicit: false,
        api_key: None,
        api_key_from_token_file: false,
    }
}

pub(crate) fn session<'session>(
    context: &'session CliContext,
    environment: &'session FakeEnvironment,
    vault: &'session mut FakeVault,
    out: &Captured,
    err: &Captured,
) -> Session<'session> {
    let client = Arc::new(Client::new(context, environment).unwrap());
    let streams = Streams::new(Box::new(out.clone()), Box::new(err.clone()));
    Session::new(SessionParts {
        context,
        environment,
        vault,
        streams,
        client,
        clock: Arc::new(CliClock),
        base_env: EnvVars::new(),
        stdin_is_terminal: false,
    })
    .unwrap()
}
