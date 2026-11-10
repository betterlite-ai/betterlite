use std::error::Error as _;
use std::sync::Arc;

use super::support::{Fixture, Script};
use crate::workload_identity::auth::WorkloadIdentityAuth;
use crate::workload_identity::cache::AUTH_CACHE_ENTRIES;
use crate::workload_identity::config::{OpenAiWorkloadIdentityConfig, SdkCapability};
use crate::workload_identity::error::WorkloadIdentityError;

const UPGRADE_MESSAGE: &str = "OpenAI workload identity federation requires openai>=2.32.0. Upgrade the installed openai package to use OPENAI_IDENTITY_PROVIDER_ID / OPENAI_SERVICE_ACCOUNT_ID / OPENAI_IDENTITY_TOKEN_FILE.";
const AUTH_IMPORT: &str = "cannot import WorkloadIdentityAuth from openai.auth";
const K8S_IMPORT: &str = "cannot import k8s_service_account_token_provider from openai.auth";

type Entry = (
    OpenAiWorkloadIdentityConfig,
    Arc<WorkloadIdentityAuth<Script>>,
);

// a lookup touches the entry, so the order of calls below is part of each scenario
fn same(fixture: &Fixture, entry: &Entry) -> bool {
    let auth = fixture
        .credentials
        .workload_identity_auth(&entry.0)
        .unwrap();
    Arc::ptr_eq(&auth, &entry.1)
}

fn filled() -> (Fixture, Vec<Entry>) {
    let fixture = Fixture::new(Vec::new());
    let entries = (0..AUTH_CACHE_ENTRIES)
        .map(|index| {
            let config = fixture.config(&format!("n{index}"), "subject");
            let auth = fixture.credentials.workload_identity_auth(&config).unwrap();
            (config, auth)
        })
        .collect();
    (fixture, entries)
}

pub(super) fn identity_cache_boundary() {
    each_identity_field_is_part_of_the_key();
    least_recently_created_entry_is_evicted();
    touched_entry_survives_eviction();
}

fn each_identity_field_is_part_of_the_key() {
    let fixture = Fixture::new(Vec::new());
    let config = fixture.config("a", "subject");
    let auth = fixture.credentials.workload_identity_auth(&config).unwrap();
    let entry = (config.clone(), auth);
    assert!(same(&fixture, &entry));
    let variants = [
        OpenAiWorkloadIdentityConfig {
            identity_provider_id: "other".to_owned(),
            ..config.clone()
        },
        OpenAiWorkloadIdentityConfig {
            service_account_id: "other".to_owned(),
            ..config.clone()
        },
        OpenAiWorkloadIdentityConfig {
            token_file: "/other/token".to_owned(),
            ..config
        },
    ];
    for variant in variants {
        let renamed = (variant, Arc::clone(&entry.1));
        assert!(!same(&fixture, &renamed));
    }
}

fn least_recently_created_entry_is_evicted() {
    let (fixture, entries) = filled();
    for entry in &entries {
        assert!(same(&fixture, entry));
    }
    let extra = fixture.config("n16", "subject");
    fixture.credentials.workload_identity_auth(&extra).unwrap();
    for entry in entries.iter().skip(1) {
        assert!(same(&fixture, entry));
    }
    assert!(!same(&fixture, &entries[0]));
}

fn touched_entry_survives_eviction() {
    let (fixture, entries) = filled();
    assert!(same(&fixture, &entries[0]));
    let extra = fixture.config("n16", "subject");
    fixture.credentials.workload_identity_auth(&extra).unwrap();
    assert!(same(&fixture, &entries[0]));
    assert!(!same(&fixture, &entries[1]));
}

const NO_SDK: SdkCapability = SdkCapability {
    workload_identity_auth: false,
    k8s_token_provider: false,
};
const NO_AUTH: SdkCapability = SdkCapability {
    workload_identity_auth: false,
    k8s_token_provider: true,
};
const NO_K8S: SdkCapability = SdkCapability {
    workload_identity_auth: true,
    k8s_token_provider: false,
};

pub(super) fn sdk_auth_unavailable_500() {
    let cases = [
        (NO_SDK, AUTH_IMPORT),
        (NO_AUTH, AUTH_IMPORT),
        (NO_K8S, K8S_IMPORT),
    ];
    for (sdk, import_failure) in cases {
        let fixture = Fixture::with_sdk(Vec::new(), sdk);
        let config = fixture.config("sdk", "subject");
        let failure = fixture
            .credentials
            .workload_identity_auth(&config)
            .err()
            .unwrap();
        let WorkloadIdentityError::SdkUnavailable { provider, cause } = &failure else {
            panic!("expected SdkUnavailable, got {failure:?}");
        };
        assert_eq!(provider.status_code, 500);
        assert_eq!(provider.message, UPGRADE_MESSAGE);
        assert_eq!(failure.to_string(), UPGRADE_MESSAGE);
        assert_eq!(cause.to_string(), import_failure);
        assert_eq!(failure.source().unwrap().to_string(), import_failure);
        assert_eq!(fixture.script.sent(), 0);
    }
    capability_checks();
}

fn capability_checks() {
    let config = Fixture::new(Vec::new()).config("sdk", "subject");
    let missing_k8s = config.to_sdk_workload_identity(NO_K8S);
    assert_eq!(missing_k8s.unwrap_err().to_string(), UPGRADE_MESSAGE);
    config.to_sdk_workload_identity(NO_AUTH).unwrap();
    NO_K8S.load_workload_identity_auth().unwrap();
    NO_AUTH.load_workload_identity_auth().unwrap_err();
    let names = [
        "OPENAI_IDENTITY_PROVIDER_ID",
        "OPENAI_SERVICE_ACCOUNT_ID",
        "OPENAI_IDENTITY_TOKEN_FILE",
        "openai>=2.32.0",
    ];
    for name in names {
        assert!(UPGRADE_MESSAGE.contains(name));
    }
}
