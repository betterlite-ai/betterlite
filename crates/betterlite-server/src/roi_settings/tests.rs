#![cfg(test)]
use super::*;

#[derive(Default)]
struct EmptyCatalog;

impl RouterCatalog for EmptyCatalog {
    fn model_names(&self) -> Vec<String> {
        Vec::new()
    }

    fn deployments(&self, _: &str) -> Vec<RouterEstimatorDeployment> {
        Vec::new()
    }
}

#[tokio::test]
async fn oneshot_get_requires_authentication() {
    use tower::ServiceExt as _;

    let state = RoiSettingsState::new(
        None,
        Arc::new(EmptyCatalog),
        CipherSettings {
            signing_key: None,
            algorithm: betterlite_secrets::secret_codec::Algorithm::XSalsa20Poly1305,
        },
    );
    let response = router(state)
        .oneshot(
            http::Request::builder()
                .uri("/roi-calculator/settings")
                .body(axum::body::Body::empty())
                .expect("valid request"),
        )
        .await
        .expect("router response");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
