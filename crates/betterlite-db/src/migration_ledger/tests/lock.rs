use tokio::time::sleep;

#[tokio::test]
async fn coordinator_lock_contention() {
    database(|database| async move {
        let holder = connection(&database.pool).await;
        holder.batch_execute("BEGIN").await.unwrap();
        assert!(xact_probe(&holder, MIGRATION_LOCK_KEY).await);
        let watcher = connection(&database.pool).await;
        assert!(!xact_probe(&watcher, MIGRATION_LOCK_KEY).await);
        let pending = connection(&database.pool).await;
        let jitter = DrawingJitter(0);
        let acquiring = MigrationCoordinator::acquire(pending, Duration::from_secs(20), &jitter);
        let releasing = async {
            sleep(Duration::from_millis(300)).await;
            holder.batch_execute("COMMIT").await.unwrap();
        };
        let (acquired, ()) = tokio::join!(acquiring, releasing);
        let coordinator = acquired.unwrap();
        assert!(!xact_probe(&watcher, MIGRATION_LOCK_KEY).await);
        coordinator.finish().await.unwrap();
        assert!(xact_probe(&watcher, MIGRATION_LOCK_KEY).await);
    })
    .await;
}

#[tokio::test]
async fn lock_timeout_deadline() {
    database(|database| async move {
        let holder = connection(&database.pool).await;
        holder.batch_execute("BEGIN").await.unwrap();
        assert!(xact_probe(&holder, MIGRATION_LOCK_KEY).await);
        let pending = connection(&database.pool).await;
        let jitter = DrawingJitter(0);
        let started = Instant::now();
        let error = MigrationCoordinator::acquire(pending, Duration::from_millis(300), &jitter)
            .await
            .expect_err("the coordinator lock stays held");
        let elapsed = started.elapsed();
        assert!(
            matches!(error, LockError::CoordinatorTimeout { .. }),
            "{error:?}"
        );
        assert!(
            error.to_string().contains("LITELLM_MIGRATION_LOCK_TIMEOUT"),
            "{error}"
        );
        assert!(elapsed >= Duration::from_millis(300), "{elapsed:?}");
        assert!(elapsed < Duration::from_secs(30), "{elapsed:?}");
        holder.batch_execute("ROLLBACK").await.unwrap();
        let free = connection(&database.pool).await;
        let coordinator = MigrationCoordinator::acquire(free, Duration::from_millis(300), &jitter)
            .await
            .unwrap();
        coordinator.finish().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn prisma_xact_lock_contention() {
    database(|database| async move {
        let coordinator = acquired_coordinator(&database.pool, Duration::from_millis(300)).await;
        let jitter = DrawingJitter(0);
        let holder = connection(&database.pool).await;
        holder.batch_execute("BEGIN").await.unwrap();
        assert!(xact_probe(&holder, PRISMA_LOCK_KEY).await);
        let error = coordinator
            .acquire_prisma_lock(&jitter)
            .await
            .expect_err("Prisma's lock is held elsewhere");
        assert!(matches!(error, LockError::PrismaLockTimeout), "{error:?}");
        assert!(error.to_string().contains("Prisma's lock"), "{error}");
        holder.batch_execute("COMMIT").await.unwrap();
        coordinator.acquire_prisma_lock(&jitter).await.unwrap();
        assert!(!xact_probe(&holder, PRISMA_LOCK_KEY).await);
        coordinator.finish().await.unwrap();
        assert!(xact_probe(&holder, PRISMA_LOCK_KEY).await);
    })
    .await;
}

#[tokio::test]
async fn connection_check_before_commit() {
    database(|database| async move {
        let coordinator = acquired_coordinator(&database.pool, Duration::from_secs(10)).await;
        coordinator
            .client
            .batch_execute("CREATE TABLE rolled_back (id int)")
            .await
            .unwrap();
        assert!(
            coordinator
                .client
                .batch_execute("SELECT 1 / 0")
                .await
                .is_err()
        );
        let error = coordinator
            .finish()
            .await
            .expect_err("the check connection statement fails first");
        assert!(matches!(error, LockError::Connection { .. }), "{error:?}");
        assert!(!exists(&database.pool, "rolled_back").await);
    })
    .await;
}

#[tokio::test]
async fn lock_connection_error() {
    database(|database| async move {
        let doomed = connection(&database.pool).await;
        let pid: i32 = doomed
            .query_one("SELECT pg_backend_pid()", &[])
            .await
            .unwrap()
            .get(0);
        let killer = connection(&database.pool).await;
        let terminated: bool = killer
            .query_one("SELECT pg_terminate_backend($1) AS terminated", &[&pid])
            .await
            .unwrap()
            .get(0);
        assert!(terminated);
        sleep(Duration::from_millis(200)).await;
        let jitter = DrawingJitter(0);
        let error = MigrationCoordinator::acquire(doomed, Duration::from_secs(10), &jitter)
            .await
            .expect_err("the terminated connection cannot start a transaction");
        assert!(matches!(error, LockError::Connection { .. }), "{error:?}");
        assert!(StdError::source(&error).is_some());
    })
    .await;
}

#[tokio::test]
async fn held_lock_unlock_on_error() {
    database(|database| async move {
        let worker = connection(&database.pool).await;
        let outcome: Result<(), HeldLockError<BodyError>> =
            held_migration_lock(&worker, |acquired| async move {
                assert!(acquired);
                Err(BodyError::Refused)
            })
            .await;
        assert!(matches!(
            outcome,
            Err(HeldLockError::Body(BodyError::Refused))
        ));
        let probe = connection(&database.pool).await;
        assert!(session_probe(&probe, MIGRATION_LOCK_KEY).await);
        session_unlock(&probe, MIGRATION_LOCK_KEY).await;
    })
    .await;
}

#[tokio::test]
async fn held_lock_false() {
    database(|database| async move {
        let holder = connection(&database.pool).await;
        assert!(session_probe(&holder, MIGRATION_LOCK_KEY).await);
        let worker = connection(&database.pool).await;
        let outcome: Result<bool, HeldLockError<BodyError>> =
            held_migration_lock(&worker, |acquired| async move { Ok(acquired) }).await;
        assert!(!outcome.unwrap());
        let probe = connection(&database.pool).await;
        assert!(!session_probe(&probe, MIGRATION_LOCK_KEY).await);
        session_unlock(&holder, MIGRATION_LOCK_KEY).await;
        assert!(session_probe(&probe, MIGRATION_LOCK_KEY).await);
        session_unlock(&probe, MIGRATION_LOCK_KEY).await;
    })
    .await;
}

#[test]
fn pause_floor_and_deadline() {
    let floor = DrawingJitter(0);
    assert_eq!(pause(&floor, None), Duration::from_millis(500));
    let sooner = TokioInstant::now()
        .checked_add(Duration::from_millis(40))
        .unwrap();
    assert!(pause(&floor, Some(sooner)) <= Duration::from_millis(40));
    let drawn = DrawingJitter(250_000_000);
    assert_eq!(pause(&drawn, None), Duration::from_millis(750));
}

#[test]
fn system_jitter_draws_inside_the_range() {
    let jitter = SystemJitter::try_new().unwrap();
    assert_eq!(jitter.uniform_nanos(0), 0);
    let draws: Vec<u64> = repeat_with(|| jitter.uniform_nanos(2)).take(256).collect();
    assert!(draws.iter().all(|draw| *draw < 2));
    assert!(draws.contains(&0));
    assert!(draws.contains(&1));
}
