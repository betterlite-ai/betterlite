use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use std::{env, fs, process};

use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::client::sessions::client::{SessionClient, SessionSettings};
use crate::client::sessions::negotiate::{
    Callbacks, ClientSession, InitializeOutcome, InitializeRequest, ProtocolVersion,
};
use crate::client::sessions::run::run_with_session;
use crate::client::sessions::runtime::{McpRuntime as _, SessionOptions};
use crate::client::sessions::stdio::StdioStreams;
use crate::client::sessions::stdio_runtime::{StdioRuntime, StdioSessionFactory};
use crate::client::sessions::transport::{
    StdioConfig, StdioServerParameters, TransportContext as _, TransportType,
};
use crate::credentials::{McpAuthType, McpClient as CredentialClient};

use super::fixture::tls_policy;

struct TestInit {
    version: &'static str,
}

impl InitializeOutcome for TestInit {
    fn protocol_version(&self) -> &str {
        self.version
    }

    fn instructions(&self) -> Option<&str> {
        None
    }
}

struct TestSession {
    _streams: StdioStreams,
}

impl ClientSession for TestSession {
    type Error = io::Error;
    type InitializeResult = TestInit;

    fn initialize(&mut self) -> impl Future<Output = Result<TestInit, io::Error>> + Send {
        let init = TestInit {
            version: "2025-06-18",
        };
        ready(Ok(init))
    }

    fn send_initialize(
        &mut self,
        _request: InitializeRequest,
    ) -> impl Future<Output = Result<TestInit, io::Error>> + Send {
        let init = TestInit {
            version: "2025-06-18",
        };
        ready(Ok(init))
    }

    fn adopt(&mut self, _result: &TestInit) {}

    fn send_initialized(&mut self) -> impl Future<Output = Result<(), io::Error>> + Send {
        ready(Ok(()))
    }
}

#[derive(Default)]
struct TestFactory {
    pid: Arc<Mutex<Option<u32>>>,
}

impl StdioSessionFactory for TestFactory {
    type Session = TestSession;

    fn build(&self, streams: StdioStreams, _options: SessionOptions<io::Error>) -> TestSession {
        *self.pid.lock().unwrap_or_else(PoisonError::into_inner) = Some(streams.pid);
        TestSession { _streams: streams }
    }
}

fn parameters(
    command: &str,
    args: Vec<String>,
    env: BTreeMap<String, String>,
) -> StdioServerParameters {
    StdioServerParameters {
        command: command.to_owned(),
        args,
        env,
        defaults: BTreeMap::new(),
    }
}

fn process_exists(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

#[tokio::test]
async fn the_stdio_server_dies_when_its_owner_exits() {
    let runtime = StdioRuntime {
        factory: Arc::new(TestFactory::default()),
    };
    let mut transport = runtime.stdio_client(parameters(
        "/bin/sleep",
        vec!["60".to_owned()],
        BTreeMap::new(),
    ));
    let streams = transport.enter().await.unwrap();
    let pid = streams.pid;
    drop(streams);
    transport.exit().await.unwrap();
    assert!(!process_exists(pid));
}

#[tokio::test]
async fn the_stdio_server_dies_when_its_owner_is_dropped() {
    let runtime = StdioRuntime {
        factory: Arc::new(TestFactory::default()),
    };
    let mut transport = runtime.stdio_client(parameters(
        "/bin/sleep",
        vec!["60".to_owned()],
        BTreeMap::new(),
    ));
    let streams = transport.enter().await.unwrap();
    let pid = streams.pid;
    drop(transport);
    drop(streams);
    sleep(Duration::from_millis(500)).await;
    assert!(!process_exists(pid));
}

fn settings(script: String) -> SessionSettings {
    SessionSettings {
        transport_type: TransportType::Stdio,
        stdio: Some(StdioConfig {
            command: "/bin/sh".to_owned(),
            args: vec!["-c".to_owned(), script],
            env: Some(BTreeMap::from([("API_KEY".to_owned(), "x".to_owned())])),
        }),
        environment: BTreeMap::from([("PATH".to_owned(), "/usr/bin:/bin".to_owned())]),
        npm_cache_dir: "/cache".to_owned(),
        tls: tls_policy(),
        protocol_version: ProtocolVersion::Auto,
        client_version: "1.105.1".to_owned(),
        callbacks: Callbacks::default(),
    }
}

fn session_client(
    settings: SessionSettings,
    runtime: StdioRuntime<TestFactory>,
) -> SessionClient<StdioRuntime<TestFactory>> {
    let credentials = CredentialClient::new(
        String::new(),
        McpAuthType::None,
        None,
        None,
        BTreeMap::new(),
        Duration::from_secs(60),
        None,
        None,
    );
    SessionClient {
        runtime: Arc::new(runtime),
        auth: credentials,
        settings,
        tracker: TaskTracker::new(),
        caller_cancelled: CancellationToken::new(),
        last_initialize_instructions: None,
    }
}

#[tokio::test]
async fn a_stdio_server_that_traps_term_finishes_before_the_kill() {
    let owner = process::id();
    let marker = env::temp_dir().join(format!("betterlite-b4744-stdio-term-{owner}.marker"));
    drop(fs::remove_file(&marker));
    let marker_path = marker.display().to_string();
    let script = format!(
        "trap 'printf done > {marker_path}; exit 0' TERM; while :; do /bin/sleep 0.1; done"
    );
    let runtime = StdioRuntime {
        factory: Arc::new(TestFactory::default()),
    };
    let mut transport = runtime.stdio_client(parameters(
        "/bin/sh",
        vec!["-c".to_owned(), script],
        BTreeMap::new(),
    ));
    let streams = transport.enter().await.unwrap();
    let pid = streams.pid;
    drop(streams);
    transport.exit().await.unwrap();
    assert!(marker.exists(), "the server did not finish on SIGTERM");
    assert!(!process_exists(pid));
    drop(fs::remove_file(&marker));
}

#[tokio::test]
async fn a_stdio_server_that_exits_on_stdin_eof_finishes_before_the_kill() {
    let owner = process::id();
    let marker = env::temp_dir().join(format!("betterlite-b4744-stdio-{owner}.marker"));
    drop(fs::remove_file(&marker));
    let marker_path = marker.display().to_string();
    let runtime = StdioRuntime {
        factory: Arc::new(TestFactory::default()),
    };
    let pid = Arc::clone(&runtime.factory.pid);
    let mut client = session_client(
        settings(format!(
            "set -e; cat > /dev/null; printf done > {marker_path}"
        )),
        runtime,
    );
    run_with_session(&mut client, async |_session| Ok("done"), false)
        .await
        .unwrap();
    let started = pid
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .expect("the server started");
    assert!(marker.exists(), "the server did not finish on stdin EOF");
    assert!(!process_exists(started));
    drop(fs::remove_file(&marker));
}
