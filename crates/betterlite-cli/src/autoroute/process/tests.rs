#![cfg(test)]

mod support;

use std::cell::Cell;
use std::fmt::Write as _;
use std::fs;
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use rstest::rstest;
use socket2::{Domain, Protocol, SockAddr, Socket, Type};

use self::support::{
    FakeClock, FakeLiveliness, FakeProcess, FakeRuntime, FixedProbe, ScriptedSignals,
};
use super::control::{
    Clock, PathSignals, RealClock, SignalOutcome, SignalPort as _, TermSignal, terminate,
};
use super::error::ProcessError;
use super::launcher::{
    ExitCode, LaunchRequest, ProcessLauncher as _, SetprivLauncher, gateway_command,
};
use super::liveliness::poll_liveliness;
use super::log_tail::stream_log;
use super::pid_record::{PidRecord, read_pid_record, write_pid_record};
use super::port::{LoopbackPortProbe, PortProbe as _};
use super::runtime::{GATEWAY_INSTALL_STEP, gateway_runtime_available};

#[test]
fn gateway_runtime_available_returns_an_executable_that_runs() {
    let runtime = FakeRuntime {
        found: Some(PathBuf::from("/opt/betterlite")),
        runs: true,
    };
    assert_eq!(
        gateway_runtime_available(&runtime).unwrap(),
        PathBuf::from("/opt/betterlite")
    );
}

#[test]
fn gateway_runtime_available_names_the_install_step_when_the_executable_is_missing() {
    let runtime = FakeRuntime {
        found: None,
        runs: false,
    };
    let error = gateway_runtime_available(&runtime).unwrap_err();
    assert!(error.to_string().contains(GATEWAY_INSTALL_STEP), "{error}");
}

#[test]
fn gateway_runtime_available_rejects_an_executable_that_does_not_run() {
    let runtime = FakeRuntime {
        found: Some(PathBuf::from("/opt/betterlite")),
        runs: false,
    };
    let error = gateway_runtime_available(&runtime).unwrap_err();
    assert!(error.to_string().contains("/opt/betterlite"), "{error}");
    assert!(error.to_string().contains(GATEWAY_INSTALL_STEP), "{error}");
}

#[test]
fn port_probe_sets_reuseaddr_before_binding_the_loopback_port() {
    let holder = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP)).unwrap();
    holder.set_reuse_address(true).unwrap();
    let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0);
    holder.bind(&SockAddr::from(address)).unwrap();
    let port = holder.local_addr().unwrap().as_socket().unwrap().port();
    // A second bind succeeds only because both sockets set SO_REUSEADDR and neither listens
    assert!(LoopbackPortProbe.is_available(port));
}

#[test]
fn port_probe_reports_false_when_a_listening_socket_holds_the_port() {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    assert!(!LoopbackPortProbe.is_available(port));
}

#[cfg(unix)]
#[test]
fn launcher_wraps_the_gateway_in_setpriv_with_the_death_signal() {
    let executable = PathBuf::from("/opt/betterlite");
    let arguments = ["--config".to_owned(), "/c".to_owned()];
    let log_path = PathBuf::from("/l");
    let request = LaunchRequest {
        executable: &executable,
        arguments: &arguments,
        log_path: &log_path,
    };
    let command = gateway_command(&request);
    assert_eq!(command.get_program().to_str().unwrap(), "setpriv");
    let rendered: Vec<&str> = command
        .get_args()
        .map(|argument| argument.to_str().unwrap())
        .collect();
    assert_eq!(
        rendered,
        [
            "--pdeathsig",
            "KILL",
            "--",
            "/opt/betterlite",
            "--config",
            "/c"
        ]
    );
}

#[cfg(unix)]
#[test]
fn gateway_dies_with_owner_when_its_handle_drops() {
    let directory = tempfile::TempDir::new().unwrap();
    let log_path = directory.path().join("proxy.log");
    let executable = PathBuf::from("sleep");
    let arguments = ["60".to_owned()];
    let launcher = SetprivLauncher;
    let process = launcher
        .launch(&LaunchRequest {
            executable: &executable,
            arguments: &arguments,
            log_path: &log_path,
        })
        .unwrap();
    let pid = process.pid();
    assert!(PathSignals.is_running(pid));
    assert!(log_path.exists());
    drop(process);
    assert!(!PathSignals.is_running(pid));
}

#[test]
fn pid_record_round_trips_through_json() {
    let directory = tempfile::TempDir::new().unwrap();
    let path = directory.path().join("proxy.pid.json");
    let record = PidRecord {
        pid: 4242,
        port: 5483,
        config_path: PathBuf::from("/home/tester/.litellm/autorouter/config.yaml"),
        log_path: PathBuf::from("/home/tester/.litellm/autorouter/proxy.log"),
    };
    write_pid_record(&path, &record).unwrap();
    assert_eq!(read_pid_record(&path).unwrap(), Some(record));
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("\"pid\": 4242"), "{text}");
}

#[test]
fn pid_record_read_returns_none_when_the_file_is_missing() {
    let directory = tempfile::TempDir::new().unwrap();
    let path = directory.path().join("proxy.pid.json");
    assert_eq!(read_pid_record(&path).unwrap(), None);
}

#[test]
fn pid_record_read_ignores_unknown_keys() {
    let directory = tempfile::TempDir::new().unwrap();
    let path = directory.path().join("proxy.pid.json");
    fs::write(
        &path,
        r#"{"pid": 7, "port": 5483, "config_path": "/c", "log_path": "/l", "extra": true}"#,
    )
    .unwrap();
    let record = read_pid_record(&path).unwrap().unwrap();
    assert_eq!(record.pid, 7);
}

#[rstest]
#[case::not_json("not json")]
#[case::missing_field(r#"{"pid": 4}"#)]
#[case::wrong_type(r#"{"pid": "four", "port": 5483, "config_path": "/c", "log_path": "/l"}"#)]
#[case::zero_pid(r#"{"pid": 0, "port": 5483, "config_path": "/c", "log_path": "/l"}"#)]
#[case::negative_pid(r#"{"pid": -1, "port": 5483, "config_path": "/c", "log_path": "/l"}"#)]
#[case::port_out_of_range(r#"{"pid": 4, "port": 70000, "config_path": "/c", "log_path": "/l"}"#)]
fn pid_record_read_reports_a_corrupt_record(#[case] content: &str) {
    let directory = tempfile::TempDir::new().unwrap();
    let path = directory.path().join("proxy.pid.json");
    fs::write(&path, content).unwrap();
    let error = read_pid_record(&path).unwrap_err();
    assert!(
        matches!(error, ProcessError::CorruptPidRecord { .. }),
        "{error}"
    );
    assert!(
        error.to_string().contains("invalid or unexpected JSON"),
        "{error}"
    );
}

#[rstest]
#[case::exists(SignalOutcome::Exists, true)]
#[case::denied(SignalOutcome::Denied, true)]
#[case::missing(SignalOutcome::Missing, false)]
fn is_running_reports_false_only_when_signal_zero_says_the_process_is_missing(
    #[case] outcome: SignalOutcome,
    #[case] expected: bool,
) {
    assert_eq!(FixedProbe(outcome).is_running(4242), expected);
}

#[cfg(unix)]
#[test]
fn is_running_treats_a_pid_of_zero_as_gone() {
    assert!(!PathSignals.is_running(0));
    assert!(!PathSignals.is_running(u32::MAX));
}

#[derive(Clone, Copy)]
enum SignalScript {
    ExitsOnTerm,
    IgnoresTerm,
    AlreadyGone,
}

fn scripted(script: SignalScript) -> ScriptedSignals {
    let (alive, exits_on_term) = match script {
        SignalScript::ExitsOnTerm => (true, true),
        SignalScript::IgnoresTerm => (true, false),
        SignalScript::AlreadyGone => (false, false),
    };
    ScriptedSignals {
        alive: Mutex::new(alive),
        exits_on_term,
        sent: Mutex::new(Vec::new()),
    }
}

#[test]
fn sigterm_sigkill_escalates_after_the_grace_period() {
    let signals = scripted(SignalScript::IgnoresTerm);
    let clock = FakeClock::default();
    terminate(4242, &signals, &clock);
    assert_eq!(signals.sent(), [TermSignal::Term, TermSignal::Kill]);
    assert_eq!(clock.sleeps(), 25);
    assert!(!signals.alive());
}

#[test]
fn sigterm_sigkill_stops_after_sigterm_when_the_process_exits() {
    let signals = scripted(SignalScript::ExitsOnTerm);
    let clock = FakeClock::default();
    terminate(4242, &signals, &clock);
    assert_eq!(signals.sent(), [TermSignal::Term]);
    assert_eq!(clock.sleeps(), 0);
}

#[test]
fn sigterm_sigkill_skips_a_process_that_is_already_gone() {
    let signals = scripted(SignalScript::AlreadyGone);
    let clock = FakeClock::default();
    terminate(4242, &signals, &clock);
    assert!(signals.sent().is_empty());
    assert_eq!(clock.sleeps(), 0);
}

fn liveliness(statuses: Vec<bool>, answer_when_empty: bool) -> FakeLiveliness {
    FakeLiveliness {
        statuses: Mutex::new(statuses),
        answer_when_empty,
        urls: Mutex::new(Vec::new()),
    }
}

fn running_process() -> FakeProcess {
    FakeProcess {
        pid: 4242,
        exit: None,
        checks: Cell::new(0),
    }
}

#[test]
fn poll_liveliness_returns_when_the_endpoint_answers_200() {
    let directory = tempfile::TempDir::new().unwrap();
    let log_path = directory.path().join("proxy.log");
    let mut process = running_process();
    let probe = liveliness(vec![false, false, true], false);
    let clock = FakeClock::default();
    poll_liveliness(
        "http://127.0.0.1:5483/",
        &log_path,
        &mut process,
        &probe,
        &clock,
    )
    .unwrap();
    assert_eq!(clock.sleeps(), 2);
    let urls = probe.urls();
    assert_eq!(urls.len(), 3);
    assert_eq!(urls[0], "http://127.0.0.1:5483/health/liveliness");
}

#[test]
fn poll_liveliness_reports_an_early_exit_with_the_last_log_lines() {
    let directory = tempfile::TempDir::new().unwrap();
    let log_path = directory.path().join("proxy.log");
    fs::write(&log_path, "starting\nboom\n").unwrap();
    let mut process = FakeProcess {
        pid: 4242,
        exit: Some(ExitCode { code: Some(3_i32) }),
        checks: Cell::new(0),
    };
    let probe = liveliness(Vec::new(), false);
    let clock = FakeClock::default();
    let error = poll_liveliness(
        "http://127.0.0.1:5483",
        &log_path,
        &mut process,
        &probe,
        &clock,
    )
    .unwrap_err();
    let text = error.to_string();
    assert!(text.contains("exited early (code 3)"), "{text}");
    assert!(text.contains("boom"), "{text}");
}

#[test]
fn poll_liveliness_times_out_after_thirty_seconds_with_the_log_tail() {
    let directory = tempfile::TempDir::new().unwrap();
    let log_path = directory.path().join("proxy.log");
    let rows = (1_u8..=45).fold(String::new(), |mut text, row| {
        let _appended = writeln!(text, "row-{row:02}");
        text
    });
    fs::write(&log_path, rows).unwrap();
    let mut process = running_process();
    let probe = liveliness(Vec::new(), false);
    let clock = FakeClock::default();
    let error = poll_liveliness(
        "http://127.0.0.1:5483",
        &log_path,
        &mut process,
        &probe,
        &clock,
    )
    .unwrap_err();
    let text = error.to_string();
    assert!(text.contains("within 30s"), "{text}");
    assert!(text.contains("row-45"), "{text}");
    assert!(text.contains("row-06"), "{text}");
    assert!(!text.contains("row-05"), "{text}");
    assert_eq!(clock.sleeps(), 60);
}

#[test]
fn poll_liveliness_reports_a_missing_log_file() {
    let directory = tempfile::TempDir::new().unwrap();
    let log_path = directory.path().join("proxy.log");
    let mut process = running_process();
    let probe = liveliness(Vec::new(), false);
    let clock = FakeClock::default();
    let error = poll_liveliness(
        "http://127.0.0.1:5483",
        &log_path,
        &mut process,
        &probe,
        &clock,
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("(no log output captured)"),
        "{error}"
    );
}

struct StopAtFirstSleep<'flag> {
    flag: &'flag AtomicBool,
}

impl Clock for StopAtFirstSleep<'_> {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn sleep(&self, _duration: Duration) {
        self.flag.store(true, Ordering::SeqCst);
    }
}

#[test]
fn stream_log_prints_the_lines_it_tails_until_stopped() {
    let directory = tempfile::TempDir::new().unwrap();
    let log_path = directory.path().join("proxy.log");
    fs::write(&log_path, "first\nsecond\n").unwrap();
    let stop = AtomicBool::new(false);
    let mut output = Vec::new();
    stream_log(
        &log_path,
        &stop,
        &StopAtFirstSleep { flag: &stop },
        &mut output,
    );
    assert_eq!(String::from_utf8(output).unwrap(), "first\nsecond\n");
}

#[test]
fn stream_log_returns_without_output_when_stopped_before_the_file_appears() {
    let directory = tempfile::TempDir::new().unwrap();
    let log_path = directory.path().join("proxy.log");
    let stop = AtomicBool::new(true);
    let mut output = Vec::new();
    stream_log(&log_path, &stop, &RealClock, &mut output);
    assert!(output.is_empty());
}
