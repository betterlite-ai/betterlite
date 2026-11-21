#[test]
fn masking_masks_every_sensitive_keyword() {
    let masked = mask_of(
        r#"{"authorization":"abcdefgh","token":"abcdefgh","key":"abcdefgh","secret":"abcdefgh","vertex_credentials":"abcdefgh","credentials":"abcdefgh","password":"abcdefgh","passwd":"abcdefgh"}"#,
    );
    assert_eq!(text(field(&masked, "authorization")), "ab****gh");
    assert_eq!(text(field(&masked, "token")), "ab****gh");
    assert_eq!(text(field(&masked, "key")), "ab****gh");
    assert_eq!(text(field(&masked, "secret")), "ab****gh");
    assert_eq!(text(field(&masked, "vertex_credentials")), "ab****gh");
    assert_eq!(text(field(&masked, "credentials")), "ab****gh");
    assert_eq!(text(field(&masked, "password")), "ab****gh");
    assert_eq!(text(field(&masked, "passwd")), "ab****gh");
}

#[test]
fn masking_matches_a_sensitive_keyword_inside_a_name() {
    let masked = mask_of(
        r#"{"aws_session_token":"abcdefgh","api_key":"abcdefgh","max_tokens":"abcdefgh","guardrail_secret_name":"abcdefgh","client_email":"abcdefgh","mode":"pre_call"}"#,
    );
    assert_eq!(text(field(&masked, "aws_session_token")), "ab****gh");
    assert_eq!(text(field(&masked, "api_key")), "ab****gh");
    assert_eq!(text(field(&masked, "max_tokens")), "ab****gh");
    assert_eq!(text(field(&masked, "guardrail_secret_name")), "ab****gh");
    assert_eq!(text(field(&masked, "client_email")), "abcdefgh");
    assert_eq!(text(field(&masked, "mode")), "pre_call");
}

#[test]
fn masking_matches_a_sensitive_keyword_in_any_case() {
    let masked = mask_of(r#"{"API_KEY":"abcdefgh","Secret":"abcdefgh"}"#);
    assert_eq!(text(field(&masked, "API_KEY")), "ab****gh");
    assert_eq!(text(field(&masked, "Secret")), "ab****gh");
}

#[test]
fn masking_replaces_short_strings_with_five_asterisks() {
    let masked = mask_of(r#"{"key":"abcd","api_key":"abc","secret":""}"#);
    assert_eq!(text(field(&masked, "key")), "*****");
    assert_eq!(text(field(&masked, "api_key")), "*****");
    assert_eq!(text(field(&masked, "secret")), "*****");
}

#[test]
fn masking_keeps_two_characters_on_each_side_of_four_asterisks() {
    let masked = mask_of(r#"{"key":"abcde","api_key":"abcdefgh"}"#);
    assert_eq!(text(field(&masked, "key")), "ab****de");
    assert_eq!(text(field(&masked, "api_key")), "ab****gh");
}

#[test]
fn masking_keeps_non_string_values_and_lists() {
    let input = r#"{"key":123,"token":true,"secret":null,"credentials":[{"password":"abcdefgh"}],"passwd":{"a":1}}"#;
    assert_eq!(mask_of(input), parse(input));
}

#[test]
fn masking_masks_an_object_under_a_sensitive_key() {
    let masked = mask_of(r#"{"credentials":{"client_email":"a@b.c","private_key":"abcdefgh"}}"#);
    let credentials = field(&masked, "credentials");
    assert_eq!(text(field(credentials, "client_email")), "a@b.c");
    assert_eq!(text(field(credentials, "private_key")), "ab****gh");
}

#[test]
fn masking_leaves_a_nested_object_under_another_key_alone() {
    let input = r#"{"config":{"api_key":"abcdefgh"}}"#;
    assert_eq!(mask_of(input), parse(input));
}

#[test]
fn masking_counts_characters_not_bytes() {
    let masked = mask_of("{\"key\":\"ééab\",\"token\":\"ééabcde\"}");
    assert_eq!(text(field(&masked, "key")), "*****");
    assert_eq!(text(field(&masked, "token")), "éé****de");
}

#[test]
fn masking_masks_objects_below_depth_twenty() {
    let chain = nested_secret(21);
    let masked = JsonValue::Object(mask_sensitive_values(&chain));
    assert_eq!(innermost(&masked), "s3****er");
}

#[test]
fn masking_returns_the_object_at_depth_twenty() {
    let chain = nested_secret(22);
    let masked = JsonValue::Object(mask_sensitive_values(&chain));
    assert_eq!(innermost(&masked), "s3cret-number");
}
