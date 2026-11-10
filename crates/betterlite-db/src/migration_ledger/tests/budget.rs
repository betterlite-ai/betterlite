#[test]
fn attempt_and_contention_budgets() {
    let budget = MigrateAttemptBudget::default();
    assert_eq!(budget.attempts_left, MAX_MIGRATE_DEPLOY_ATTEMPTS);
    assert_eq!(budget.contention_left, DEFAULT_LOCK_TIMEOUT);
    assert_eq!(budget.attempt_number(), 1_i64);
    let spent = budget.spend();
    assert_eq!(spent.attempts_left, MAX_MIGRATE_DEPLOY_ATTEMPTS - 1_i32);
    assert_eq!(spent.contention_left, DEFAULT_LOCK_TIMEOUT);
    assert_eq!(spent.attempt_number(), 2_i64);
    let contended = spent.after_contention(Duration::from_secs(100)).unwrap();
    assert_eq!(contended.attempts_left, spent.attempts_left);
    assert_eq!(contended.contention_left, Duration::from_secs(500));
    contended
        .after_contention(Duration::from_secs(500))
        .unwrap_err();
    contended
        .after_contention(Duration::from_secs(501))
        .unwrap_err();
    assert_eq!(
        contended
            .after_contention(Duration::from_secs(499))
            .unwrap()
            .contention_left,
        Duration::from_secs(1)
    );
    let exhausted = budget.spend().spend().spend().spend();
    assert_eq!(exhausted.attempts_left, 0_i32);
    assert_eq!(exhausted.attempt_number(), 5_i64);
}

#[test]
fn exhausted_attempts_keep_decrementing() {
    let exhausted = MigrateAttemptBudget::default()
        .spend()
        .spend()
        .spend()
        .spend();
    let overspent = exhausted.spend();
    assert_eq!(overspent.attempts_left, -1_i32);
    assert!(overspent.attempts_left <= 0_i32);
    assert_eq!(overspent.attempt_number(), 6_i64);
    let further = overspent.spend();
    assert_eq!(further.attempts_left, -2_i32);
    assert_eq!(further.attempt_number(), 7_i64);
    assert_eq!(
        further
            .after_contention(Duration::from_secs(1))
            .unwrap()
            .attempts_left,
        -2_i32
    );
}
