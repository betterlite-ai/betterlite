#[tokio::test]
async fn setup_schema_failure_closes_fixture_leaving_database_unreachable() {
    let [first, mut second] = Box::pin(provision_all()).await;
    let urls = [first.options.url.clone(), second.options.url.clone()];
    second.options.url = SecretString::from("invalid connection string".to_owned());
    let result = Box::pin(harness_on(
        [first, second],
        |[_first, _second]| async {
            panic!("body must not run after setup fails");
        },
        BODY_TIMEOUT,
    ))
    .await;
    assert!(matches!(result, Err(HarnessError::Database(_))));
    assert_databases_removed(urls).await;
}

#[tokio::test]
async fn schema_setup_preserves_timeout_source() {
    let fixture = Box::pin(provision()).await.unwrap();
    let result =
        initialize_schema(&fixture, "SELECT pg_sleep(60)", Duration::from_millis(50)).await;
    close_fixtures(vec![fixture]).await;
    let error = result.unwrap_err();
    let source = error.source().unwrap();
    assert!(source.downcast_ref::<Elapsed>().is_some());
}

#[tokio::test]
async fn schema_setup_preserves_database_error_source() {
    let fixture = Box::pin(provision()).await.unwrap();
    let result = initialize_schema(&fixture, "not valid sql;", SETUP_TIMEOUT).await;
    close_fixtures(vec![fixture]).await;
    let error = result.unwrap_err();
    let source = error.source().unwrap();
    assert_eq!(
        source
            .downcast_ref::<tokio_postgres::Error>()
            .unwrap()
            .code(),
        Some(&SqlState::SYNTAX_ERROR),
    );
}

#[rstest]
#[case::timeout(false)]
#[case::panic(true)]
#[tokio::test]
async fn body_failure_closes_both_databases(#[case] should_panic: bool) {
    let fixtures = Box::pin(provision_all::<2>()).await;
    let urls = fixtures
        .each_ref()
        .map(|fixture| fixture.options.url.clone());
    let (sender, mut receiver) = oneshot::channel();
    let outcome = Box::pin(harness_on(
        fixtures,
        move |[client_a, client_b]| async move {
            assert!(!should_panic, "deliberate body failure");
            exec(&client_a, "SELECT 1").await;
            client_b.batch_execute("SELECT pg_sleep(60)").await.unwrap();
            sender.send(()).unwrap();
        },
        Duration::from_millis(50),
    ))
    .await;
    assert!(matches!(receiver.try_recv(), Err(TryRecvError::Closed)));
    assert_databases_removed(urls).await;
    if should_panic {
        assert!(matches!(outcome, Err(HarnessError::Body(error)) if error.is_panic()));
    } else {
        assert!(matches!(outcome, Err(HarnessError::Timeout(_))));
    }
}

async fn assert_databases_removed(urls: [SecretString; 2]) {
    for url in urls {
        let reconnect = timeout(
            SETUP_TIMEOUT,
            tokio_postgres::connect(url.expose_secret(), tokio_postgres::NoTls),
        )
        .await
        .unwrap();
        assert!(
            reconnect.is_err(),
            "fixture database survived harness cleanup"
        );
    }
}
use std::error::Error as _;
use tokio::sync::oneshot;
use tokio::sync::oneshot::error::TryRecvError;
