#![cfg(test)]

use crate::assistants::client::{ClientKind, ClientOptions};
use crate::assistants::records::ListParams;

use crate::assistants::operations::AssistantsApi;

use super::support::{PAGE, Recorder, api_with, options};

#[tokio::test]
async fn openai_assistants_credential_identity_isolation() {
    let (api, transport) = api_with(200, PAGE);
    transport.queue(200, PAGE);
    let tenant_a = options("sk-tenant-a");
    let tenant_b = ClientOptions {
        api_base: Some(String::from("https://b.test/v1")),
        organization: None,
        ..options("sk-tenant-b")
    };
    let client_a = api.get_openai_client(ClientKind::Sync, tenant_a.clone(), None);
    let client_b = api.get_openai_client(ClientKind::Sync, tenant_b.clone(), None);
    assert_eq!(client_a.organization.as_deref(), Some("org-fixture"));
    assert_eq!(client_b.organization, None);
    let kept = api.get_openai_client(ClientKind::Async, tenant_b, Some(client_a));
    assert_eq!(kept.base_url.as_deref(), Some("https://example.test/v1"));
    api.get_assistants(tenant_a, None, false, &ListParams::default())
        .await
        .unwrap();
    let rebuilt = ClientOptions {
        api_key: client_b.api_key,
        api_base: client_b.base_url,
        timeout: None,
        max_retries: None,
        organization: None,
    };
    api.get_assistants(rebuilt, None, false, &ListParams::default())
        .await
        .unwrap();
    let seen = transport.requests();
    assert_eq!(seen[0].authorization.as_deref(), Some("Bearer sk-tenant-a"));
    assert_eq!(seen[0].organization.as_deref(), Some("org-fixture"));
    assert!(seen[0].url.starts_with("https://example.test/"));
    assert_eq!(seen[1].authorization.as_deref(), Some("Bearer sk-tenant-b"));
    assert_eq!(seen[1].organization, None);
    assert!(seen[1].url.starts_with("https://b.test/"));
}

#[tokio::test]
async fn openai_assistants_secret_safe_failure() {
    let secret = "sk-secret-fixture-value";
    let (api, transport) = api_with(401, r#"{"error":{"message":"bad key"}}"#);
    let configured = options(secret);
    assert!(!format!("{configured:?}").contains(secret));
    let client = api.get_openai_client(ClientKind::Sync, configured, None);
    assert!(!format!("{client:?}").contains(secret));
    let rejected = client.list(Vec::new()).await.unwrap_err();
    let missing_key = ClientOptions {
        api_key: None,
        ..options(secret)
    };
    let bad_base = ClientOptions {
        api_base: Some(String::from("not a url")),
        ..options(secret)
    };
    let mut shown = vec![format!("{rejected} {rejected:?}")];
    shown.push(failure_text(&api, missing_key).await);
    shown.push(failure_text(&api, bad_base).await);
    shown.push(failure_text(&api, options(&format!("{secret}\n"))).await);
    shown.extend(transport.debug_texts());
    assert_eq!(shown.len(), 5);
    assert!(shown.iter().all(|text| !text.contains(secret)), "{shown:?}");
}

async fn failure_text(api: &AssistantsApi<Recorder>, broken: ClientOptions) -> String {
    let error = api
        .get_openai_client(ClientKind::Sync, broken, None)
        .list(Vec::new())
        .await
        .unwrap_err();
    format!("{error} {error:?}")
}
