use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};
use std::{env, fs, process};

use tokio::io::AsyncReadExt as _;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::client::sdk_runtime::handler::ServerRequestHandlers;
use crate::client::sessions::client::{SessionClient, SessionSettings};
use crate::client::sessions::negotiate::{
    Callbacks, ClientSession, InitializeOutcome, InitializeRequest, ProtocolVersion,
};
use crate::client::sessions::run::run_with_session;
use crate::client::sessions::runtime::{McpRuntime as _, SessionOptions};
use crate::client::sessions::stdio::{
    FORCE_KILL_TIMEOUT, SDK_SHUTDOWN, StdioShutdown, StdioStreams,
};
use crate::client::sessions::stdio_runtime::{StdioRuntime, StdioSessionFactory};
use crate::client::sessions::transport::{
    StdioConfig, StdioServerParameters, TransportContext as _, TransportType,
};
use crate::credentials::{McpAuthType, McpClient as CredentialClient};

use super::fixture::tls_policy;

// These servers set no trap or print ready once their traps are set, so SIGTERM may come soon after stdin closes
const QUICK_TERMINATION: Duration = Duration::from_millis(100);

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

fn process_running(pid: u32) -> bool {
    let Ok(status) = fs::read_to_string(format!("/proc/{pid}/status")) else {
        return false;
    };
    status
        .lines()
        .find_map(|line| line.strip_prefix("State:"))
        .is_some_and(|state| !state.trim_start().starts_with('Z'))
}

#[tokio::test]
async fn a_stdio_server_child_that_ignores_term_is_killed_after_the_leader_exits() {
    let owner = process::id();
    let pid_file = env::temp_dir().join(format!("betterlite-b4744-child-{owner}.pid"));
    drop(fs::remove_file(&pid_file));
    let pid_path = pid_file.display().to_string();
    let script = format!(
        "/bin/sh -c 'trap \"\" TERM; printf ready; while :; do /bin/sleep 0.1; done' & echo $! > {pid_path}; trap 'exit 0' TERM; wait"
    );
    let runtime = StdioRuntime {
        factory: Arc::new(TestFactory::default()),
        shutdown: StdioShutdown {
            termination: QUICK_TERMINATION,
            force_kill: Duration::from_millis(300),
        },
    };
    let mut transport = runtime.stdio_client(parameters(
        "/bin/sh",
        vec!["-c".to_owned(), script],
        BTreeMap::new(),
    ));
    let mut streams = transport.enter().await.unwrap();
    let leader = streams.pid;
    let mut ready = [0_u8; 5];
    streams.stdout.read_exact(&mut ready).await.unwrap();
    drop(streams);
    wait_for(
        || fs::read_to_string(&pid_file).is_ok_and(|pid| pid.ends_with('\n')),
        "the child wrote its pid",
    )
    .await;
    let child = fs::read_to_string(&pid_file).expect("the child wrote its pid");
    let child_pid = child.trim().parse::<u32>().expect("the pid is a number");
    assert!(process_running(child_pid));
    transport.exit().await.unwrap();
    wait_for(|| !process_exists(leader), "the leader outlived exit").await;
    wait_for(
        || !process_running(child_pid),
        "the descendant survived the owner",
    )
    .await;
    drop(fs::remove_file(&pid_file));
}

#[tokio::test]
async fn a_leader_that_exits_first_still_sweeps_its_surviving_child() {
    let owner = process::id();
    let pid_file = env::temp_dir().join(format!("betterlite-b4744-survivor-{owner}.pid"));
    drop(fs::remove_file(&pid_file));
    let pid_path = pid_file.display().to_string();
    let script = format!("/bin/sleep 60 & echo $! > {pid_path}; exit 0");
    let runtime = StdioRuntime {
        factory: Arc::new(TestFactory::default()),
        shutdown: StdioShutdown {
            termination: QUICK_TERMINATION,
            force_kill: FORCE_KILL_TIMEOUT,
        },
    };
    let mut transport = runtime.stdio_client(parameters(
        "/bin/sh",
        vec!["-c".to_owned(), script],
        BTreeMap::new(),
    ));
    let streams = transport.enter().await.unwrap();
    let leader = streams.pid;
    drop(streams);
    wait_for(
        || fs::read_to_string(&pid_file).is_ok_and(|pid| pid.ends_with('\n')),
        "the child wrote its pid",
    )
    .await;
    let child = fs::read_to_string(&pid_file).expect("the child wrote its pid");
    let child_pid = child.trim().parse::<u32>().expect("the pid is a number");
    assert!(process_running(child_pid));
    transport.exit().await.unwrap();
    wait_for(|| !process_exists(leader), "the leader outlived exit").await;
    wait_for(
        || !process_running(child_pid),
        "the descendant outlived the leader",
    )
    .await;
    drop(fs::remove_file(&pid_file));
}

#[tokio::test]
async fn a_stdio_server_that_ignores_term_is_killed_after_the_force_window() {
    let runtime = StdioRuntime {
        factory: Arc::new(TestFactory::default()),
        shutdown: StdioShutdown {
            termination: QUICK_TERMINATION,
            force_kill: Duration::from_millis(300),
        },
    };
    let script = "trap '' TERM; printf ready; while :; do /bin/sleep 0.1; done";
    let mut transport = runtime.stdio_client(parameters(
        "/bin/sh",
        vec!["-c".to_owned(), script.to_owned()],
        BTreeMap::new(),
    ));
    let mut streams = transport.enter().await.unwrap();
    let pid = streams.pid;
    let mut ready = [0_u8; 5];
    streams.stdout.read_exact(&mut ready).await.unwrap();
    drop(streams);
    transport.exit().await.unwrap();
    assert!(!process_exists(pid));
}

#[tokio::test]
async fn the_stdio_server_dies_when_its_owner_exits() {
    let runtime = StdioRuntime {
        factory: Arc::new(TestFactory::default()),
        shutdown: StdioShutdown {
            termination: QUICK_TERMINATION,
            force_kill: FORCE_KILL_TIMEOUT,
        },
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
        shutdown: SDK_SHUTDOWN,
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
    wait_for(|| !process_exists(pid), "the server outlived its owner").await;
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
        handlers: ServerRequestHandlers::default(),
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
        "trap 'printf done > {marker_path}; exit 0' TERM; printf ready; while :; do /bin/sleep 0.1; done"
    );
    let runtime = StdioRuntime {
        factory: Arc::new(TestFactory::default()),
        shutdown: StdioShutdown {
            termination: QUICK_TERMINATION,
            force_kill: FORCE_KILL_TIMEOUT,
        },
    };
    let mut transport = runtime.stdio_client(parameters(
        "/bin/sh",
        vec!["-c".to_owned(), script],
        BTreeMap::new(),
    ));
    let mut streams = transport.enter().await.unwrap();
    let pid = streams.pid;
    let mut ready = [0_u8; 5];
    streams.stdout.read_exact(&mut ready).await.unwrap();
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
        shutdown: SDK_SHUTDOWN,
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

// the server and its children are other processes, so their exit is polled on the wall clock
async fn wait_for<F>(condition: F, failure: &str)
where
    F: Fn() -> bool,
{
    let started = Instant::now();
    while !condition() {
        assert!(started.elapsed() < Duration::from_secs(30), "{failure}");
        sleep(Duration::from_millis(10)).await;
    }
}
