#![cfg(test)]

use super::{
    AGENTIC_LOOP_KWARG_NAMES, BEDROCK_BATCH_KWARG_NAMES, Field, INTERNAL_KWARG_PREFIX, Owner,
    Record, WireMetadata, is_litellm_owned_kwarg, owned_names, owned_wire_names, wire_names,
};

const BATCH: &str = "aws_batch_role_arn s3_bucket_name s3_region_name s3_endpoint_url s3_output_bucket_name s3_bucket_owner s3_access_key_id s3_secret_access_key s3_encryption_key_id bedrock_tags";
const CONNECTION: &str = "api_key api_base api_version region_name headers provider_specific_header client shared_session ssl_verify request_timeout force_timeout stream_timeout max_retries tenant_id client_id client_secret azure_username azure_password azure_scope azure_ad_token_provider litellm_credential_name configurable_clientside_auth_params use_xai_oauth";
const AGENTIC: &str = "_agentic_loop_depth _agentic_loop_fingerprints _agentic_loop_api_surface _code_interpreter_interception_active _code_interpreter_interception_sandbox_key _code_interpreter_interception_session_scoped _code_interpreter_interception_converted_stream _websearch_interception_emit_native_blocks _websearch_interception_converted_stream _headroom_interception_converted_stream max_agentic_loops";

#[rstest::rstest]
#[case::wire_name_projection(wire_name_projection)]
#[case::owned_name_registry(owned_name_registry)]
#[case::internal_prefix_membership(internal_prefix_membership)]
#[case::subset_registries(subset_registries)]
fn owned_parameter_names_cases(#[case] assertions: fn()) {
    assertions();
}

fn wire_name_projection() {
    assert_eq!(
        wire_names(Owner::ConnectionSettings),
        ["provider", "bedrock_batch"]
    );
    assert_eq!(wire_names(Owner::ObservabilityOptions), "id metadata litellm_metadata tags litellm_trace_id litellm_session_id litellm_request_debug logger_fn verbose no-log log_client_error_tracebacks".split_whitespace().collect::<Vec<_>>());
    assert_eq!(
        owned_wire_names(Owner::ConnectionSettings.record()).unwrap(),
        CONNECTION
            .split_whitespace()
            .chain(BATCH.split_whitespace())
            .collect::<Vec<_>>()
    );
    let fields = [Field {
        name: "nested",
        metadata: None,
        leaf: Some(Owner::ConnectionSettings),
    }; 2];
    assert_eq!(
        owned_wire_names(Record {
            name: "Nested",
            fields: &fields
        })
        .unwrap(),
        ["provider", "bedrock_batch", "provider", "bedrock_batch"]
    );
    let error = owned_wire_names(Owner::ProviderConnection.record()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "ProviderConnection.api_key is not a dataclass leaf"
    );
    assert!(
        owned_wire_names(Record {
            name: "Empty",
            fields: &[]
        })
        .unwrap()
        .is_empty()
    );
}

fn owned_name_registry() {
    let names = owned_names();
    let expected = include_str!("owned_names.txt")
        .split_whitespace()
        .collect::<Vec<_>>();
    assert_eq!(names, expected);
    for name in expected {
        assert!(is_litellm_owned_kwarg(name), "{name}");
    }
}

fn internal_prefix_membership() {
    for name in [
        INTERNAL_KWARG_PREFIX,
        "_litellm_fixture",
        "allowed_openai_params",
        "no-log",
    ] {
        assert!(is_litellm_owned_kwarg(name), "{name}");
    }
    for name in [
        "litellm_fixture",
        "ordinary_parameter",
        "API_KEY",
        "no_log",
        "",
        " _litellm_",
        "_LITELLM_",
    ] {
        assert!(!is_litellm_owned_kwarg(name), "{name}");
    }
}

fn subset_registries() {
    assert_eq!(
        AGENTIC_LOOP_KWARG_NAMES.names(),
        AGENTIC.split_whitespace().collect::<Vec<_>>()
    );
    assert_eq!(
        BEDROCK_BATCH_KWARG_NAMES.names(),
        BATCH.split_whitespace().collect::<Vec<_>>()
    );
}

#[test]
fn owned_parameter_names_credential_identity_isolation() {
    let before = owned_names();
    let identities = ["identity_a-fixture-secret", "identity_b-fixture-secret"];
    let classified = identities.map(is_litellm_owned_kwarg);
    assert_eq!(classified, [false, false]);
    let [mut first, mut second] = identities.map(|name| {
        let mut local = owned_names();
        local.push(name.to_owned());
        local
    });
    first.extend(["identity_a-header", "identity_a-cache"].map(str::to_owned));
    second.extend(["identity_b-header", "identity_b-cache"].map(str::to_owned));
    assert_ne!(first, second);
    assert!(!first.iter().any(|name| name.starts_with("identity_b")));
    assert!(!second.iter().any(|name| name.starts_with("identity_a")));
    assert_eq!(owned_names(), before);
    let debug = format!("{classified:?} {:?}", owned_names());
    assert!(!debug.contains("identity_a"));
    assert!(!debug.contains("identity_b"));
}

#[test]
fn owned_parameter_names_secret_safe_failure() {
    let before = owned_names();
    let credentials = ["identity_a-fixture-secret", "identity_b-fixture-secret"];
    let errors = credentials.map(|credential| {
        assert!(!is_litellm_owned_kwarg(credential));
        let fields = [
            Field {
                name: "valid",
                metadata: None,
                leaf: Some(Owner::ProviderConnection),
            },
            Field {
                name: "value",
                metadata: Some(WireMetadata {
                    wire_name: credential,
                }),
                leaf: None,
            },
        ];
        owned_wire_names(Record {
            name: "InvalidLeaf",
            fields: &fields,
        })
        .unwrap_err()
    });
    assert_eq!(
        errors.each_ref().map(ToString::to_string),
        ["InvalidLeaf.value is not a dataclass leaf"; 2]
    );
    let debug = format!("{errors:?} {:?}", owned_names());
    assert!(!debug.contains("identity_a"));
    assert!(!debug.contains("identity_b"));
    assert_eq!(owned_names(), before);
}
