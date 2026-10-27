#![cfg(test)]

use std::io;
use std::sync::{Arc, Mutex};

use rstest::rstest;
use tracing::subscriber::with_default;

use crate::s3_transport::encryption::resolve_server_side_encryption;
use crate::s3_transport::option_value::OptionValue;

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[rstest]
#[case::nothing_configured("null", "null", "none")]
#[case::algorithm_only("AES256", "null", "AES256 None")]
#[case::key_implies_kms("null", "'key'", "aws:kms Some(\"key\")")]
#[case::blank_algorithm_with_key("''", "'key'", "aws:kms Some(\"key\")")]
#[case::kms_keeps_key("'aws:kms'", "'key'", "aws:kms Some(\"key\")")]
#[case::dsse_keeps_key("'aws:kms:dsse'", "'key'", "aws:kms:dsse Some(\"key\")")]
#[case::aes_drops_key("AES256", "'key'", "AES256 None warned")]
#[case::kms_with_blank_key("'aws:kms'", "''", "aws:kms None")]
#[case::blank_both("''", "''", "none")]
#[case::blank_key_only("null", "''", "none")]
#[case::non_string_algorithm_dropped("5", "'key'", "aws:kms Some(\"key\") warned")]
#[case::non_string_key_dropped("AES256", "7", "AES256 None warned")]
#[case::mapping_algorithm_dropped("{}", "null", "none warned")]
fn s3_option_resolution_behaviors_behavior(
    #[case] algorithm: &str,
    #[case] kms_key_id: &str,
    #[case] expected: &str,
) {
    let algorithm_value: Option<OptionValue> = serde_saphyr::from_str(algorithm).unwrap();
    let key_value: Option<OptionValue> = serde_saphyr::from_str(kms_key_id).unwrap();
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let resolved = with_default(subscriber, || {
        resolve_server_side_encryption(algorithm_value.as_ref(), key_value.as_ref())
    });
    let output = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    let rendered = resolved.map_or_else(
        || "none".to_owned(),
        |encryption| format!("{} {:?}", encryption.algorithm, encryption.kms_key_id),
    );
    let shown = if output.contains("WARN") {
        format!("{rendered} warned")
    } else {
        rendered
    };
    assert_eq!(shown, expected, "{output}");
}
