#![cfg(test)]

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::{ServerSpec, Subject};
use crate::credential_contracts::auth_config::AuthConfig;
use crate::credential_contracts::auth_config::simple::NoneConfig;
use crate::credential_contracts::auth_spec::AuthSpecKind;

#[rstest]
fn server_spec_auth_spec_kind_derives_from_config() {
    let spec = ServerSpec {
        server_id: "srv-1".to_owned(),
        resource: "https://upstream.example.test".to_owned(),
        config: AuthConfig::None(NoneConfig),
    };
    assert_eq!(spec.config.kind(), AuthSpecKind::None);
}

#[rstest]
fn subject_carries_the_inbound_token() {
    let subject = Subject {
        tenant_id: "tenant-1".to_owned(),
        subject_id: "user-1".to_owned(),
        inbound_token: Some(SecretString::from("inbound-jwt".to_owned())),
    };
    assert_eq!(subject.tenant_id, "tenant-1");
    assert_eq!(
        subject
            .inbound_token
            .map(|token| token.expose_secret().to_owned()),
        Some("inbound-jwt".to_owned())
    );
}
