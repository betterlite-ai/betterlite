use crate::prometheus::failure::deployment_labels::{
    DeploymentFailureLabels, DeploymentFailureSources, PayloadMetadata, PayloadMetadataRecord,
    UserApiKeyFacts, deployment_failure_labels,
};
use crate::prometheus::failure::tests::support::{api_key_facts, metadata, metadata_record};

#[test]
fn the_payload_metadata_names_every_label() {
    let payload = metadata(&[
        ("user_api_key_alias", "alias"),
        ("user_api_key_user_email", "user@example.com"),
        ("user_api_key_team_id", "team-1"),
        ("user_api_key_team_alias", "Team One"),
        ("user_api_key_hash", "hash-1"),
        ("requester_ip_address", "203.0.113.7"),
        ("user_agent", "curl/8"),
    ]);
    let sources = DeploymentFailureSources {
        payload_metadata: Some(PayloadMetadata::Mapping(&payload)),
        litellm_params_metadata: None,
        user_api_key_auth: None,
    };
    let labels = deployment_failure_labels(&sources);
    let expected = DeploymentFailureLabels {
        api_key_alias: Some(String::from("alias")),
        user_email: Some(String::from("user@example.com")),
        team: Some(String::from("team-1")),
        team_alias: Some(String::from("Team One")),
        hashed_api_key: Some(String::from("hash-1")),
        client_ip: Some(String::from("203.0.113.7")),
        user_agent: Some(String::from("curl/8")),
    };
    assert_eq!(labels, expected);
}

#[test]
fn the_litellm_params_metadata_names_every_label() {
    let payload = metadata(&[]);
    let params = metadata(&[
        ("user_api_key_alias", "alias"),
        ("user_api_key_user_email", "user@example.com"),
        ("user_api_key_team_id", "team-1"),
        ("user_api_key_team_alias", "Team One"),
        ("user_api_key_hash", "hash-1"),
        ("requester_ip_address", "203.0.113.7"),
        ("user_agent", "curl/8"),
    ]);
    let sources = DeploymentFailureSources {
        payload_metadata: Some(PayloadMetadata::Mapping(&payload)),
        litellm_params_metadata: Some(&params),
        user_api_key_auth: None,
    };
    let labels = deployment_failure_labels(&sources);
    let expected = DeploymentFailureLabels {
        api_key_alias: Some(String::from("alias")),
        user_email: Some(String::from("user@example.com")),
        team: Some(String::from("team-1")),
        team_alias: Some(String::from("Team One")),
        hashed_api_key: Some(String::from("hash-1")),
        client_ip: Some(String::from("203.0.113.7")),
        user_agent: Some(String::from("curl/8")),
    };
    assert_eq!(labels, expected);
}

#[test]
fn the_auth_object_names_every_label_it_holds() {
    let payload = metadata(&[]);
    let params = metadata(&[]);
    let auth = UserApiKeyFacts {
        key_alias: Some(String::from("auth-alias")),
        user_email: Some(String::from("auth@example.com")),
        team_id: Some(String::from("team-auth")),
        team_alias: Some(String::from("Auth Team")),
        hashed_key: Some(String::from("hash-auth")),
        key_hash: None,
    };
    let sources = DeploymentFailureSources {
        payload_metadata: Some(PayloadMetadata::Mapping(&payload)),
        litellm_params_metadata: Some(&params),
        user_api_key_auth: Some(&auth),
    };
    let labels = deployment_failure_labels(&sources);
    let expected = DeploymentFailureLabels {
        api_key_alias: Some(String::from("auth-alias")),
        user_email: Some(String::from("auth@example.com")),
        team: Some(String::from("team-auth")),
        team_alias: Some(String::from("Auth Team")),
        hashed_api_key: Some(String::from("hash-auth")),
        client_ip: None,
        user_agent: None,
    };
    assert_eq!(labels, expected);
}

#[test]
fn the_payload_wins_over_the_params_and_the_auth() {
    let payload = metadata(&[("user_api_key_alias", "payload-alias")]);
    let params = metadata(&[("user_api_key_alias", "params-alias")]);
    let auth = UserApiKeyFacts {
        key_alias: Some(String::from("auth-alias")),
        ..api_key_facts()
    };
    let sources = DeploymentFailureSources {
        payload_metadata: Some(PayloadMetadata::Mapping(&payload)),
        litellm_params_metadata: Some(&params),
        user_api_key_auth: Some(&auth),
    };
    let labels = deployment_failure_labels(&sources);
    assert_eq!(labels.api_key_alias.as_deref(), Some("payload-alias"));
}

#[test]
fn the_params_win_over_the_auth() {
    let payload = metadata(&[]);
    let params = metadata(&[("user_api_key_alias", "params-alias")]);
    let auth = UserApiKeyFacts {
        key_alias: Some(String::from("auth-alias")),
        ..api_key_facts()
    };
    let sources = DeploymentFailureSources {
        payload_metadata: Some(PayloadMetadata::Mapping(&payload)),
        litellm_params_metadata: Some(&params),
        user_api_key_auth: Some(&auth),
    };
    let labels = deployment_failure_labels(&sources);
    assert_eq!(labels.api_key_alias.as_deref(), Some("params-alias"));
}

#[test]
fn an_empty_payload_value_stops_the_cascade() {
    let payload = metadata(&[("user_api_key_alias", "")]);
    let params = metadata(&[("user_api_key_alias", "params-alias")]);
    let sources = DeploymentFailureSources {
        payload_metadata: Some(PayloadMetadata::Mapping(&payload)),
        litellm_params_metadata: Some(&params),
        user_api_key_auth: None,
    };
    let labels = deployment_failure_labels(&sources);
    assert_eq!(labels.api_key_alias.as_deref(), Some(""));
}

#[test]
fn an_empty_payload_client_ip_falls_to_the_params() {
    let payload = metadata(&[("requester_ip_address", "")]);
    let params = metadata(&[("requester_ip_address", "198.51.100.9")]);
    let sources = DeploymentFailureSources {
        payload_metadata: Some(PayloadMetadata::Mapping(&payload)),
        litellm_params_metadata: Some(&params),
        user_api_key_auth: None,
    };
    let labels = deployment_failure_labels(&sources);
    assert_eq!(labels.client_ip.as_deref(), Some("198.51.100.9"));
}

#[test]
fn an_empty_payload_user_agent_falls_to_the_params() {
    let payload = metadata(&[("user_agent", "")]);
    let params = metadata(&[("user_agent", "litellm/1.0")]);
    let sources = DeploymentFailureSources {
        payload_metadata: Some(PayloadMetadata::Mapping(&payload)),
        litellm_params_metadata: Some(&params),
        user_api_key_auth: None,
    };
    let labels = deployment_failure_labels(&sources);
    assert_eq!(labels.user_agent.as_deref(), Some("litellm/1.0"));
}

#[test]
fn no_source_anywhere_is_no_label() {
    let payload = metadata(&[]);
    let params = metadata(&[]);
    let sources = DeploymentFailureSources {
        payload_metadata: Some(PayloadMetadata::Mapping(&payload)),
        litellm_params_metadata: Some(&params),
        user_api_key_auth: None,
    };
    let labels = deployment_failure_labels(&sources);
    assert_eq!(labels, DeploymentFailureLabels::default());
}

#[test]
fn an_empty_auth_hashed_key_falls_to_the_key_hash() {
    let auth = UserApiKeyFacts {
        hashed_key: Some(String::new()),
        key_hash: Some(String::from("hash-2")),
        ..api_key_facts()
    };
    let sources = DeploymentFailureSources {
        payload_metadata: None,
        litellm_params_metadata: None,
        user_api_key_auth: Some(&auth),
    };
    let labels = deployment_failure_labels(&sources);
    assert_eq!(labels.hashed_api_key.as_deref(), Some("hash-2"));
}

#[test]
fn an_absent_auth_hashed_key_falls_to_the_key_hash() {
    let auth = UserApiKeyFacts {
        key_hash: Some(String::from("hash-2")),
        ..api_key_facts()
    };
    let sources = DeploymentFailureSources {
        payload_metadata: None,
        litellm_params_metadata: None,
        user_api_key_auth: Some(&auth),
    };
    let labels = deployment_failure_labels(&sources);
    assert_eq!(labels.hashed_api_key.as_deref(), Some("hash-2"));
}

#[test]
fn the_auth_hashed_key_wins_over_the_key_hash() {
    let auth = UserApiKeyFacts {
        hashed_key: Some(String::from("hash-1")),
        key_hash: Some(String::from("hash-2")),
        ..api_key_facts()
    };
    let sources = DeploymentFailureSources {
        payload_metadata: None,
        litellm_params_metadata: None,
        user_api_key_auth: Some(&auth),
    };
    let labels = deployment_failure_labels(&sources);
    assert_eq!(labels.hashed_api_key.as_deref(), Some("hash-1"));
}

#[test]
fn a_metadata_record_names_every_label() {
    let record = PayloadMetadataRecord {
        user_api_key_alias: Some(String::from("alias")),
        user_api_key_user_email: Some(String::from("user@example.com")),
        user_api_key_team_id: Some(String::from("team-1")),
        user_api_key_team_alias: Some(String::from("Team One")),
        user_api_key_hash: Some(String::from("hash-1")),
        requester_ip_address: Some(String::from("203.0.113.7")),
        user_agent: Some(String::from("curl/8")),
    };
    let sources = DeploymentFailureSources {
        payload_metadata: Some(PayloadMetadata::Record(&record)),
        litellm_params_metadata: None,
        user_api_key_auth: None,
    };
    let labels = deployment_failure_labels(&sources);
    let expected = DeploymentFailureLabels {
        api_key_alias: Some(String::from("alias")),
        user_email: Some(String::from("user@example.com")),
        team: Some(String::from("team-1")),
        team_alias: Some(String::from("Team One")),
        hashed_api_key: Some(String::from("hash-1")),
        client_ip: Some(String::from("203.0.113.7")),
        user_agent: Some(String::from("curl/8")),
    };
    assert_eq!(labels, expected);
}

#[test]
fn a_missing_record_attribute_falls_to_the_params() {
    let params = metadata(&[("user_api_key_alias", "params-alias")]);
    let record = metadata_record();
    let sources = DeploymentFailureSources {
        payload_metadata: Some(PayloadMetadata::Record(&record)),
        litellm_params_metadata: Some(&params),
        user_api_key_auth: None,
    };
    let labels = deployment_failure_labels(&sources);
    assert_eq!(labels.api_key_alias.as_deref(), Some("params-alias"));
}
