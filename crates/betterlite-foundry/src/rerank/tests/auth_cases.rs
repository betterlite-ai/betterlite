use crate::credentials::error::CredentialsError;
use crate::rerank::error::FoundryRerankError;

use super::support::{
    FixtureSources, auth_request, environment, expected_headers, failure, headers, key, object,
    resolved,
};

pub(super) fn foundry_rerank_auth() {
    explicit_and_variable_keys();
    global_fallback();
    explicit_empty_enters_entra();
    caller_authorization();
    caller_extra_headers();
    params_only_on_the_entra_path();
    empty_token_is_missing();
}

fn explicit_and_variable_keys() {
    let sources = FixtureSources::new();
    let empty = environment(&[]);
    let env_key = environment(&[("AZURE_AI_API_KEY", "env-key")]);
    let global = key("key-global");
    let explicit = key("key-explicit");
    assert_eq!(
        resolved(&auth_request(Some(&explicit), &empty, None, None, &sources)),
        expected_headers("Bearer key-explicit")
    );
    assert_eq!(
        resolved(&auth_request(None, &env_key, None, None, &sources)),
        expected_headers("Bearer env-key")
    );
    // the variable wins over the global key
    assert_eq!(
        resolved(&auth_request(None, &env_key, Some(&global), None, &sources)),
        expected_headers("Bearer env-key")
    );
    assert!(!sources.took());
}

fn global_fallback() {
    let sources = FixtureSources::new();
    let empty = environment(&[]);
    let empty_env = environment(&[("AZURE_AI_API_KEY", "")]);
    let env_key = environment(&[("AZURE_AI_API_KEY", "env-key")]);
    let global = key("key-global");
    let explicit = key("key-explicit");
    // an empty variable counts as unset and falls to the global key
    assert_eq!(
        resolved(&auth_request(
            None,
            &empty_env,
            Some(&global),
            None,
            &sources
        )),
        expected_headers("Bearer key-global")
    );
    assert_eq!(
        resolved(&auth_request(None, &empty, Some(&global), None, &sources)),
        expected_headers("Bearer key-global")
    );
    assert_eq!(
        resolved(&auth_request(
            Some(&explicit),
            &env_key,
            Some(&global),
            None,
            &sources
        )),
        expected_headers("Bearer key-explicit")
    );
    assert!(!sources.took());
}

fn explicit_empty_enters_entra() {
    let sources = FixtureSources::new();
    let empty = environment(&[]);
    let env_key = environment(&[("AZURE_AI_API_KEY", "env-key")]);
    let global = key("key-global");
    let empty_key = key("");
    let token = object(r#"{"azure_ad_token":"entra-token"}"#);
    // an explicit empty key suppresses the variable and the global key, then falls to Entra
    assert!(matches!(
        failure(&auth_request(
            Some(&empty_key),
            &env_key,
            Some(&global),
            None,
            &sources
        )),
        FoundryRerankError::Credentials(CredentialsError::MissingCredentials { .. })
    ));
    assert_eq!(
        resolved(&auth_request(
            Some(&empty_key),
            &empty,
            None,
            Some(&token),
            &sources
        )),
        expected_headers("Bearer entra-token")
    );
    assert_eq!(
        resolved(&auth_request(None, &empty, None, Some(&token), &sources)),
        expected_headers("Bearer entra-token")
    );
    assert!(matches!(
        failure(&auth_request(None, &empty, None, None, &sources)),
        FoundryRerankError::Credentials(CredentialsError::MissingCredentials { .. })
    ));
    assert!(!sources.took());
}

fn caller_authorization() {
    let sources = FixtureSources::new();
    let empty = environment(&[]);
    let explicit = key("key-explicit");
    // caller headers merge last, so a caller Authorization wins over the resolved one
    let mut caller = auth_request(Some(&explicit), &empty, None, None, &sources);
    caller.headers = headers(&[("Authorization", "Bearer caller")]);
    assert_eq!(resolved(&caller), expected_headers("Bearer caller"));
    // a caller Authorization does not bypass credential resolution
    let mut unresolved = auth_request(None, &empty, None, None, &sources);
    unresolved.headers = headers(&[("Authorization", "Bearer caller")]);
    assert!(matches!(
        failure(&unresolved),
        FoundryRerankError::Credentials(CredentialsError::MissingCredentials { .. })
    ));
    assert!(!sources.took());
}

fn caller_extra_headers() {
    let sources = FixtureSources::new();
    let empty = environment(&[]);
    let explicit = key("key-explicit");
    let mut overridden = auth_request(Some(&explicit), &empty, None, None, &sources);
    overridden.headers = headers(&[("accept", "text/plain"), ("X-Custom", "1")]);
    assert_eq!(
        resolved(&overridden),
        headers(&[
            ("Authorization", "Bearer key-explicit"),
            ("accept", "text/plain"),
            ("content-type", "application/json"),
            ("X-Custom", "1"),
        ])
    );
    // header names stay case-sensitive, so a lowercase authorization is another header
    let mut lowercase = auth_request(Some(&explicit), &empty, None, None, &sources);
    lowercase.headers = headers(&[("authorization", "Bearer lower")]);
    assert_eq!(
        resolved(&lowercase),
        headers(&[
            ("Authorization", "Bearer key-explicit"),
            ("accept", "application/json"),
            ("authorization", "Bearer lower"),
            ("content-type", "application/json"),
        ])
    );
    assert!(!sources.took());
}

fn params_only_on_the_entra_path() {
    let sources = FixtureSources::new();
    let empty = environment(&[]);
    let explicit = key("key-explicit");
    let bad_tenant = object(r#"{"tenant_id":5}"#);
    assert_eq!(
        resolved(&auth_request(
            Some(&explicit),
            &empty,
            None,
            Some(&bad_tenant),
            &sources
        )),
        expected_headers("Bearer key-explicit")
    );
    let mut bad_params = auth_request(None, &empty, None, Some(&bad_tenant), &sources);
    bad_params.headers = headers(&[("Authorization", "Bearer caller")]);
    assert!(matches!(
        failure(&bad_params),
        FoundryRerankError::Credentials(CredentialsError::InvalidParam { field: "tenant_id" })
    ));
    assert!(!sources.took());
}

fn empty_token_is_missing() {
    let sources = FixtureSources::new();
    let empty = environment(&[]);
    let empty_token = object(r#"{"azure_ad_token":""}"#);
    assert!(matches!(
        failure(&auth_request(
            None,
            &empty,
            None,
            Some(&empty_token),
            &sources
        )),
        FoundryRerankError::Credentials(CredentialsError::MissingCredentials { .. })
    ));
    assert!(!sources.took());
}
