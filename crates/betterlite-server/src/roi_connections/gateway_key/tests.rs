#![cfg(test)]

use secrecy::ExposeSecret as _;

use crate::roi_connections::error::RoiConnectionsError;
use crate::roi_connections::gateway_key::{MISSING_KEY, gateway_key};
use crate::roi_connections::tests::support::{secret, settings};

#[test]
fn gateway_key_uses_the_estimator_key() {
    let master = secret("master-key");
    let key = gateway_key(&settings(), Some(&master)).expect("estimator key");
    assert_eq!(key.expose_secret(), "sk-estimator");
}

#[test]
fn gateway_key_falls_back_to_the_master_key() {
    let mut roi = settings();
    roi.estimator_key = secret("");
    let master = secret("master-key");
    let key = gateway_key(&roi, Some(&master)).expect("master key");
    assert_eq!(key.expose_secret(), "master-key");
}

#[test]
fn gateway_key_without_a_credential_is_a_conflict() {
    let mut roi = settings();
    roi.estimator_key = secret("");
    let error = gateway_key(&roi, None).err().expect("no credential");
    assert!(matches!(error, RoiConnectionsError::Conflict(MISSING_KEY)));
}

#[test]
fn gateway_key_treats_an_empty_master_key_as_absent() {
    let mut roi = settings();
    roi.estimator_key = secret("");
    let master = secret("");
    let error = gateway_key(&roi, Some(&master))
        .err()
        .expect("no credential");
    assert!(matches!(error, RoiConnectionsError::Conflict(MISSING_KEY)));
}
