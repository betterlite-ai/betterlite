#![cfg(test)]

use super::Op;
use crate::case::HelperCase;
use crate::helpers::Helper as _;

#[test]
fn response_auth_survives_helper() {
    let fixture =
        include_str!("../../../../cases/helpers/carried-policy-records/agent-response-auth.yaml");
    let case: HelperCase = serde_saphyr::from_str(fixture).unwrap();
    let output = Op::decode_and_run(&case.call.input).unwrap();
    assert_eq!(output["kill_switch"]["auth"]["password"], "carried-secret");
}
