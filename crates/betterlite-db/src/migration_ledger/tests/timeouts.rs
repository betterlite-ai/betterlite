#[rstest]
#[case::exponent("1e3", Duration::from_secs(1000))]
#[case::leading_dot(".5", Duration::from_millis(500))]
#[case::padded(" 5 ", Duration::from_secs(5))]
#[case::fraction("2.75", Duration::from_millis(2750))]
#[case::whole("90", Duration::from_secs(90))]
fn timeout_accepts_numeric_text(#[case] text: &str, #[case] expected: Duration) {
    assert_eq!(
        timeout_from_env("PROBE", Some(text), DEFAULT_LOCK_TIMEOUT),
        expected
    );
}

#[test]
fn timeout_huge_value_never_expires() {
    assert_eq!(
        timeout_from_env("PROBE", Some("1e300"), DEFAULT_LOCK_TIMEOUT),
        Duration::MAX
    );
}

#[rstest]
#[case::zero("0")]
#[case::negative("-4")]
#[case::infinite("inf")]
#[case::nan("NaN")]
fn unusable_timeouts_fall_back_to_the_default(#[case] text: &str) {
    assert_eq!(
        timeout_from_env("PROBE", Some(text), DEFAULT_LOCK_TIMEOUT),
        DEFAULT_LOCK_TIMEOUT
    );
}

#[test]
fn timeout_defaults_apply() {
    assert_eq!(
        timeout_from_env("PROBE", None, DEFAULT_LOCK_TIMEOUT),
        DEFAULT_LOCK_TIMEOUT
    );
    assert_eq!(
        timeout_from_env(
            PRISMA_COMMAND_TIMEOUT_ENV_VAR,
            None,
            DEFAULT_COMMAND_TIMEOUT
        ),
        DEFAULT_COMMAND_TIMEOUT
    );
    assert_eq!(
        prisma_migrate_deploy_timeout(None, None),
        DEFAULT_DEPLOY_TIMEOUT
    );
    assert_eq!(
        prisma_migrate_deploy_timeout(None, Some("900")),
        Duration::from_mins(15)
    );
    assert_eq!(
        prisma_migrate_deploy_timeout(Some("1200"), Some("900")),
        Duration::from_mins(20)
    );
}
