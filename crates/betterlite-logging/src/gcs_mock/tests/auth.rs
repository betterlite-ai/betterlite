use crate::gcs_mock::auth::{
    GcsMockAuth, MOCK_PROJECT_ID, MOCK_STORAGE_URL, MOCK_TOKEN, VertexAuth as _,
    mock_vertex_auth_methods,
};
use crate::gcs_mock::installation::Installation;

use super::support::{FakeAuth, ensure_request, token_request};

#[tokio::test]
async fn methods_delegate_to_the_original_before_install() {
    let auth = GcsMockAuth {
        real: FakeAuth::new(),
        installed: false,
    };
    let asynchronous = auth
        .ensure_access_token_async(ensure_request())
        .await
        .unwrap();
    assert_eq!(asynchronous.token, "original-token");
    assert_eq!(asynchronous.project_id, "original-project");
    let synchronous = auth.ensure_access_token(ensure_request()).unwrap();
    assert_eq!(synchronous.token, "original-token");
    assert_eq!(synchronous.project_id, "original-project");
    let token_and_url = auth.get_token_and_url(token_request()).unwrap();
    assert_eq!(token_and_url.token, "original-token");
    assert_eq!(token_and_url.url, "https://original.example.com");
    assert_eq!(auth.real.calls(), 3);
}

#[tokio::test]
async fn installed_methods_return_the_fixed_literals() {
    let mut auth = GcsMockAuth {
        real: FakeAuth::new(),
        installed: false,
    };
    assert_eq!(auth.install(), Installation::Installed);
    let asynchronous = auth
        .ensure_access_token_async(ensure_request())
        .await
        .unwrap();
    assert_eq!(asynchronous.token, MOCK_TOKEN);
    assert_eq!(asynchronous.project_id, MOCK_PROJECT_ID);
    let synchronous = auth.ensure_access_token(ensure_request()).unwrap();
    assert_eq!(synchronous.token, MOCK_TOKEN);
    assert_eq!(synchronous.project_id, MOCK_PROJECT_ID);
    let token_and_url = auth.get_token_and_url(token_request()).unwrap();
    assert_eq!(token_and_url.token, MOCK_TOKEN);
    assert_eq!(token_and_url.url, MOCK_STORAGE_URL);
    assert_eq!(auth.real.calls(), 0);
}

#[tokio::test]
async fn a_second_install_keeps_the_first_installation() {
    let mut auth = GcsMockAuth {
        real: FakeAuth::new(),
        installed: false,
    };
    assert_eq!(auth.install(), Installation::Installed);
    assert_eq!(auth.install(), Installation::AlreadyInstalled);
    let asynchronous = auth
        .ensure_access_token_async(ensure_request())
        .await
        .unwrap();
    assert_eq!(asynchronous.token, MOCK_TOKEN);
    assert_eq!(auth.real.calls(), 0);
}

#[tokio::test]
async fn mock_vertex_auth_methods_installs_the_wrapper() {
    let mut auth = mock_vertex_auth_methods(FakeAuth::new());
    assert!(auth.installed);
    assert_eq!(auth.install(), Installation::AlreadyInstalled);
    let token_and_url = auth.get_token_and_url(token_request()).unwrap();
    assert_eq!(token_and_url.url, MOCK_STORAGE_URL);
}
