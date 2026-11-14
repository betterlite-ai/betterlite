use secrecy::SecretString;

use super::support::{World, record};
use crate::up::command::run_up;
#[cfg(unix)]
use crate::up::ports::StopSource;
use crate::up::restore::RestoreOnce;
#[cfg(unix)]
use crate::up::signals::SignalStop;

const FRESH: f64 = 4_000_000_000.0;

fn prepared() -> World {
    let mut world = World::new(false);
    world.seed_token(record("sk-live", 1_000.0, Some(FRESH)));
    world.context.api_key = Some(SecretString::from("sk-live".to_owned()));
    world
}

#[test]
fn up_holds_open_until_the_stop_source_fires() {
    let mut world = prepared();
    let mut ports = world.ports();
    run_up(&mut ports).expect("the stop source ended the wait");
    drop(ports);
    assert!(world.stopped());
}

#[test]
fn up_restores_the_settings_exactly_once() {
    let mut world = prepared();
    world.seed(&world.settings_path(), r#"{"model":"opus"}"#);
    let mut ports = world.ports();
    run_up(&mut ports).expect("the session ran and restored");
    drop(ports);
    let text = world.text();
    assert_eq!(text.matches("Restored ").count(), 1);
    assert_eq!(text.matches("Nothing to restore.").count(), 0);
    assert_eq!(
        super::support::json_file(&world.settings_path()),
        serde_json::json!({"model": "opus"})
    );
}

#[test]
fn up_leaves_stderr_quiet_when_the_restore_works() {
    let mut world = prepared();
    let mut ports = world.ports();
    run_up(&mut ports).expect("the session ran and restored");
    drop(ports);
    assert_eq!(String::from_utf8_lossy(&world.stderr), "");
}

#[test]
fn dropping_the_restore_guard_restores_the_settings() {
    let world = prepared();
    world.seed(
        &world.settings_path(),
        r#"{"env":{"ANTHROPIC_AUTH_TOKEN":"sk-live"}}"#,
    );
    world.seed(
        &world.backup_path(),
        r#"{"existed": true, "content": {"model": "opus"}}"#,
    );
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    {
        let _guard = RestoreOnce {
            home: world.home_path(),
            files: &world.private_files,
            stdout: &mut stdout,
            stderr: &mut stderr,
            done: std::sync::atomic::AtomicBool::new(false),
        };
    }
    assert_eq!(
        super::support::json_file(&world.settings_path()),
        serde_json::json!({"model": "opus"})
    );
    assert_eq!(
        String::from_utf8_lossy(&stdout)
            .matches("Restored ")
            .count(),
        1
    );
}

#[cfg(unix)]
#[rstest::rstest]
#[case::sigint(signal_hook::consts::SIGINT)]
#[case::sigterm(signal_hook::consts::SIGTERM)]
fn signal_stop_wait_returns_after_a_signal(#[case] signal: i32) {
    let (installed, ready) = std::sync::mpsc::channel();
    let waiter = std::thread::spawn(move || {
        let stop = SignalStop::default();
        stop.install().expect("the signal watch installed");
        installed.send(()).expect("the test is listening");
        stop.wait();
    });
    ready.recv().expect("the signal watch installed");
    std::thread::park_timeout(std::time::Duration::from_millis(100));
    assert!(!waiter.is_finished(), "wait returned before any signal");
    let raiser = std::thread::spawn(move || {
        let _raised = signal_hook::low_level::raise(signal);
    });
    waiter.join().expect("the waiter finished");
    raiser.join().expect("the raiser finished");
}

#[cfg(unix)]
#[rstest::rstest]
#[case::sigint(signal_hook::consts::SIGINT)]
#[case::sigterm(signal_hook::consts::SIGTERM)]
fn up_restores_the_settings_after_a_real_signal(#[case] signal: i32) {
    let mut world = prepared();
    world.seed(&world.settings_path(), r#"{"model":"opus"}"#);
    let installed = world.use_real_signals();
    let raiser = std::thread::spawn(move || {
        installed.recv().expect("up installed the signal watch");
        let _raised = signal_hook::low_level::raise(signal);
    });
    let mut ports = world.ports();
    run_up(&mut ports).expect("the session ran and restored");
    drop(ports);
    raiser.join().expect("the raiser finished");
    let text = world.text();
    assert_eq!(text.matches("Restored ").count(), 1);
    assert_eq!(text.matches("Nothing to restore.").count(), 0);
    assert_eq!(
        super::support::json_file(&world.settings_path()),
        serde_json::json!({"model": "opus"})
    );
}
