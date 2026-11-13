use secrecy::SecretString;

use super::support::{World, record};
use crate::up::command::run_up;

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
