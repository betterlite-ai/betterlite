#![cfg(test)]

use secrecy::ExposeSecret as _;

use crate::credentials::CredentialSource;

use super::support::{Fixture, Transport, settings};

const AMBIENT: &[(&str, &str)] = &[("AWS_EC2_METADATA_DISABLED", "true")];

#[test]
fn credential_source_resolves_at_the_region() {
    let transport = Transport::with(&[]);
    let fixture = Fixture::new(AMBIENT, &[], &transport, None);
    let statics =
        "aws_access_key_id: static, aws_secret_access_key: secret, aws_region_name: us-west-2";
    let resolved = fixture
        .runtime
        .block_on(CredentialSource::resolve(
            &fixture.resolver(),
            settings(statics, AMBIENT),
            "eu-west-1".to_owned(),
        ))
        .expect("the static pair resolves");
    assert_eq!(resolved.access_key_id.expose_secret(), "static");
    assert_eq!(resolved.secret_access_key.expose_secret(), "secret");
    assert!(resolved.session_token.is_none());
}
