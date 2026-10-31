#![cfg(test)]

#[test]
fn unknown_operation_is_decode_error() {
    let result = serde_json::from_str::<super::Input>(
        r#"{"calls":[{"name":"typo","operation":"tem","row":{"team_id":"t"}}]}"#,
    );
    let error = result
        .err()
        .expect("unknown operation must fail input decoding");
    assert!(error.to_string().contains("unknown variant `tem`"));
}

#[test]
fn float_projection_has_own_error() {
    let mut row = serde_json::from_str(r#"{"spend":1e400}"#).unwrap();
    assert!(matches!(
        super::float_amounts(&mut row),
        Err(super::AdapterError::FloatProjection)
    ));
}
