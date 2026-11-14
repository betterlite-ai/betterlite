use std::sync::atomic::AtomicBool;
#[cfg(unix)]
use std::sync::mpsc;
#[cfg(unix)]
use std::thread;
#[cfg(unix)]
use std::time::Duration;

use secrecy::SecretString;
#[cfg(unix)]
use signal_hook::consts::{SIGINT, SIGTERM};
#[cfg(unix)]
use signal_hook::low_level::raise;

use super::support::{World, document, json_file, record, seed, utf8};
use crate::up::command::run_up;
#[cfg(unix)]
use crate::up::ports::StopSource as _;
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
    seed(&world.settings_path(), r#"{"model":"opus"}"#);
    let mut ports = world.ports();
    run_up(&mut ports).expect("the session ran and restored");
    drop(ports);
    let text = world.text();
    assert_eq!(text.matches("Restored ").count(), 1);
    assert_eq!(text.matches("Nothing to restore.").count(), 0);
    assert_eq!(
        json_file(&world.settings_path()),
        Some(document(r#"{"model":"opus"}"#))
    );
}

#[test]
fn up_leaves_stderr_quiet_when_the_restore_works() {
    let mut world = prepared();
    let mut ports = world.ports();
    run_up(&mut ports).expect("the session ran and restored");
    drop(ports);
    assert_eq!(utf8(&world.stderr), "");
}

#[test]
fn dropping_the_restore_guard_restores_the_settings() {
    let world = prepared();
    seed(
        &world.settings_path(),
        r#"{"env":{"ANTHROPIC_AUTH_TOKEN":"sk-live"}}"#,
    );
    seed(
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
            done: AtomicBool::new(false),
        };
    }
    assert_eq!(
        json_file(&world.settings_path()),
        Some(document(r#"{"model":"opus"}"#))
    );
    assert_eq!(utf8(&stdout).matches("Restored ").count(), 1);
}

#[test]
fn dropping_the_restore_guard_reports_a_corrupt_backup_on_stderr() {
    let world = prepared();
    seed(
        &world.settings_path(),
        r#"{"env":{"ANTHROPIC_AUTH_TOKEN":"sk-live"}}"#,
    );
    seed(&world.backup_path(), "{not json");
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    {
        let _guard = RestoreOnce {
            home: world.home_path(),
            files: &world.private_files,
            stdout: &mut stdout,
            stderr: &mut stderr,
            done: AtomicBool::new(false),
        };
    }
    let text = utf8(&stderr);
    assert!(text.contains("invalid or unexpected JSON"), "{text}");
    assert_eq!(
        json_file(&world.settings_path()),
        Some(document(r#"{"env":{"ANTHROPIC_AUTH_TOKEN":"sk-live"}}"#))
    );
}

#[cfg(unix)]
#[rstest::rstest]
#[case::sigint(SIGINT)]
#[case::sigterm(SIGTERM)]
fn signal_stop_wait_returns_after_a_signal(#[case] signal: i32) {
    let (installed, ready) = mpsc::sync_channel(1);
    thread::scope(|scope| {
        let waiter = scope.spawn(move || {
            let stop = SignalStop::default();
            stop.install().expect("the signal watch installed");
            installed.send(()).expect("the test is listening");
            stop.wait();
        });
        ready
            .recv_timeout(Duration::from_secs(10))
            .expect("the signal watch installed");
        thread::park_timeout(Duration::from_millis(100));
        let returned_early = waiter.is_finished();
        let raiser = scope.spawn(move || {
            let _raised = raise(signal);
        });
        waiter.join().expect("the waiter finished");
        raiser.join().expect("the raiser finished");
        assert!(!returned_early, "wait returned before any signal");
    });
}

#[cfg(unix)]
#[rstest::rstest]
#[case::sigint(SIGINT)]
#[case::sigterm(SIGTERM)]
fn up_restores_the_settings_after_a_real_signal(#[case] signal: i32) {
    let mut world = prepared();
    seed(&world.settings_path(), r#"{"model":"opus"}"#);
    let installed = world.use_real_signals();
    thread::scope(|scope| {
        let raiser = scope.spawn(move || {
            installed
                .recv_timeout(Duration::from_secs(10))
                .expect("up installed the signal watch");
            let _raised = raise(signal);
        });
        let mut ports = world.ports();
        run_up(&mut ports).expect("the session ran and restored");
        drop(ports);
        raiser.join().expect("the raiser finished");
    });
    let text = world.text();
    assert_eq!(text.matches("Restored ").count(), 1);
    assert_eq!(text.matches("Nothing to restore.").count(), 0);
    assert_eq!(
        json_file(&world.settings_path()),
        Some(document(r#"{"model":"opus"}"#))
    );
}
