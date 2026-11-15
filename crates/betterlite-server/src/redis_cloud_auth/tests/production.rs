#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn production_cloud_redis_auth_returns_none_without_a_provider() {
    let environment = BTreeMap::new();
    let config = ConfigLoader {
        lookup: environment.clone(),
    }
    .load("")
    .unwrap();
    let source = production_cloud_redis_auth(&config, &environment, proxied_client())
        .await
        .unwrap();
    assert!(source.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn production_cloud_redis_auth_builds_a_source_for_a_configured_provider() {
    let environment = BTreeMap::from([(
        "REDIS_GCP_SERVICE_ACCOUNT".to_owned(),
        "sa@project.iam.gserviceaccount.com".to_owned(),
    )]);
    let config = ConfigLoader {
        lookup: environment.clone(),
    }
    .load("")
    .unwrap();
    let source = production_cloud_redis_auth(&config, &environment, proxied_client())
        .await
        .unwrap();
    assert!(source.is_some());
}
