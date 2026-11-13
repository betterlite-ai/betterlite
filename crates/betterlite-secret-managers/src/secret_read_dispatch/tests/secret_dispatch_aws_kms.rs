use crate::secret_read_dispatch::error::SecretDispatchError;
use crate::secret_read_dispatch::get_secret_from_manager;
use crate::secret_types::KeyManagementSystem;

use super::support::{assert_secret, aws_kms_client, aws_kms_fake, base64_of, environment};

#[test]
fn aws_kms_decrypts_and_strips_the_plaintext() {
    let fake = aws_kms_fake(Some(b"  plain-text \n"));
    let client = aws_kms_client(&fake);
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::AwsKms),
        "encrypted",
        None,
        &environment("encrypted", &base64_of("  plain-text \n")),
    );
    assert_secret(result, "plain-text");
    assert_eq!(fake.blobs(), [b"  plain-text \n".to_vec()]);
}

#[test]
fn aws_kms_strips_python_control_whitespace() {
    let fake = aws_kms_fake(Some(b"\x1cplain\x1f"));
    let client = aws_kms_client(&fake);
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::AwsKms),
        "encrypted",
        None,
        &environment("encrypted", &base64_of("\u{1c}plain\u{1f}")),
    );
    assert_secret(result, "plain");
}

#[test]
fn aws_kms_accepts_python_lenient_base64() {
    let fake = aws_kms_fake(Some(b"plain"));
    let client = aws_kms_client(&fake);
    let encoded = format!("{}\n", base64_of("plain"));
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::AwsKms),
        "encrypted",
        None,
        &environment("encrypted", &encoded),
    );
    assert_secret(result, "plain");
    assert_eq!(fake.blobs(), [b"plain".to_vec()]);
}

#[test]
fn aws_kms_rejects_unpadded_ciphertext() {
    let fake = aws_kms_fake(Some(b"plain"));
    let client = aws_kms_client(&fake);
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::AwsKms),
        "encrypted",
        None,
        &environment("encrypted", "abc"),
    );
    assert!(matches!(result, Err(SecretDispatchError::AwsKmsBase64(_))));
    assert!(fake.blobs().is_empty());
}

#[test]
fn aws_kms_plaintext_must_be_utf8() {
    let client = aws_kms_client(&aws_kms_fake(Some(&[0xff])));
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::AwsKms),
        "encrypted",
        None,
        &environment("encrypted", &base64_of("whatever")),
    );
    assert!(matches!(result, Err(SecretDispatchError::AwsKmsUtf8(_))));
}

#[test]
fn aws_kms_client_failure_propagates() {
    let client = aws_kms_client(&aws_kms_fake(None));
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::AwsKms),
        "encrypted",
        None,
        &environment("encrypted", &base64_of("plain")),
    );
    assert!(matches!(result, Err(SecretDispatchError::AwsKms(_))));
}
