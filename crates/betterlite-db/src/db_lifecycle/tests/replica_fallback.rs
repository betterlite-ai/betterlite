use super::support::capture_logs;
use crate::db_lifecycle::replica_fallback::initialize_reader;

#[test]
fn unset_replica_runs_writer_only_without_a_line() {
    let (logs, guard) = capture_logs();
    let reader = initialize_reader::<u32, &str>(None, None);
    drop(guard);
    assert_eq!(reader, None);
    assert!(logs.lines().is_empty(), "{:?}", logs.lines());
}

#[test]
fn reader_failure_warns_and_falls_back_to_the_writer() {
    let (logs, guard) = capture_logs();
    let reader = initialize_reader::<u32, &str>(Some(Err("no route to host")), None);
    drop(guard);
    assert_eq!(reader, None);
    let expected = "Failed to initialize read replica Prisma client: no route to host. \
                    Falling back to writer-only mode (no read routing) until proxy restart.";
    assert!(
        logs.lines().iter().any(|line| line.contains(expected)),
        "{:?}",
        logs.lines()
    );
}

#[test]
fn reader_success_enables_routing() {
    let (logs, guard) = capture_logs();
    let reader = initialize_reader::<u32, &str>(Some(Ok(41)), None);
    drop(guard);
    assert_eq!(reader, Some(41));
    assert!(
        logs.lines().iter().any(|line| line
            .contains("PrismaClient: read-replica routing enabled via DATABASE_URL_READ_REPLICA")),
        "{:?}",
        logs.lines()
    );
}

#[test]
fn token_label_is_logged_with_the_routing_line() {
    let (logs, guard) = capture_logs();
    let reader = initialize_reader::<u32, &str>(Some(Ok(41)), Some("AWS IAM"));
    drop(guard);
    assert_eq!(reader, Some(41));
    assert!(
        logs.lines().iter().any(|line| line
            .contains("PrismaClient: read-replica routing enabled via DATABASE_URL_READ_REPLICA (with AWS IAM auto-refresh)")),
        "{:?}",
        logs.lines()
    );
}
