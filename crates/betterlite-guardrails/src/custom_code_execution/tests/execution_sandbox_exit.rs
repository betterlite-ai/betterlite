use super::super::execution::error::{ExecutionError, ExitKind, SandboxError, SandboxExit};
use super::super::execution::sync::call_with_timeout;
use super::LONG;

#[test]
fn a_system_exit_is_delivered_with_its_type_and_value() {
    let result: Result<(), ExecutionError<&'static str>> = call_with_timeout(
        || Err(SandboxError::Exit(SandboxExit::system_exit("1".to_owned()))),
        LONG,
        "exit",
    );
    match result {
        Err(ExecutionError::Sandbox(SandboxError::Exit(exit))) => {
            assert_eq!(exit.kind, ExitKind::SystemExit);
            assert_eq!(exit.to_string(), "SystemExit: 1");
            assert_eq!(exit.message, "1");
        }
        other => panic!("expected the sandbox exit, got {other:?}"),
    }
}

#[test]
fn a_keyboard_interrupt_is_not_a_timeout() {
    let exit = SandboxExit {
        kind: ExitKind::KeyboardInterrupt,
        message: "user stopped it".to_owned(),
    };
    let result: Result<(), ExecutionError<&'static str>> =
        call_with_timeout(|| Err(SandboxError::Exit(exit)), LONG, "interrupt");
    match result {
        Err(ExecutionError::Sandbox(SandboxError::Exit(delivered))) => {
            assert_eq!(delivered.kind, ExitKind::KeyboardInterrupt);
            assert_eq!(delivered.to_string(), "KeyboardInterrupt: user stopped it");
        }
        other => panic!("expected the sandbox exit, got {other:?}"),
    }
}

#[test]
fn a_panicking_worker_becomes_a_panic_exit() {
    let result: Result<u8, ExecutionError<&'static str>> =
        call_with_timeout(|| panic!("worker exploded"), LONG, "panic");
    match result {
        Err(ExecutionError::Sandbox(SandboxError::Exit(exit))) => {
            assert_eq!(exit.kind, ExitKind::Panic);
            assert!(exit.to_string().starts_with("Panic: "));
        }
        other => panic!("expected a panic exit, got {other:?}"),
    }
}
