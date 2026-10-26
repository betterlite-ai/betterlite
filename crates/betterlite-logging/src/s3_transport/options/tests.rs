#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rstest::rstest;
use tracing::subscriber::with_default;

use crate::s3_transport::option_value::OptionValue;
use crate::s3_transport::options::{
    Bound, Toggle, default_max_adaptive_concurrency, resolve_bound, resolve_log_prompts_only,
    resolve_max_retry_age, resolve_partition_granularity, resolve_toggle,
};

const PROMPTS: &str = "S3_LOG_PROMPTS_ONLY";
const PARTITION: &str = "S3_PARTITION_GRANULARITY";
const CEILING: &str = "DEFAULT_S3_MAX_ADAPTIVE_CONCURRENCY";
const UPLOADS: Resolver = Resolver::Bound(Bound::MaxConcurrentUploads);
const QUEUE: Resolver = Resolver::Bound(Bound::MaxQueueSize);
const ADAPTIVE_CEILING: Resolver = Resolver::Bound(Bound::MaxAdaptiveConcurrency);
const DROP: Resolver = Resolver::Toggle(Toggle::DropOnTerminalError);
const ADAPTIVE: Resolver = Resolver::Toggle(Toggle::AdaptiveConcurrency);
const BATCH: Resolver = Resolver::Toggle(Toggle::BatchFileUpload);

#[derive(Clone, Copy)]
enum Resolver {
    PromptsOnly,
    Partition,
    Bound(Bound),
    RetryAge,
    Toggle(Toggle),
    CeilingDefault,
}

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

fn render(
    resolver: Resolver,
    value: Option<&OptionValue>,
    environment: &BTreeMap<String, String>,
) -> String {
    match resolver {
        Resolver::PromptsOnly => format!("{:?}", resolve_log_prompts_only(value, environment)),
        Resolver::Partition => {
            format!("{:?}", resolve_partition_granularity(value, environment))
        }
        Resolver::Bound(bound) => format!("{:?}", resolve_bound(bound, value, 7)),
        Resolver::RetryAge => {
            let fallback = Some(Duration::from_secs(9));
            format!("{:?}", resolve_max_retry_age(value, fallback))
        }
        Resolver::Toggle(toggle) => format!("{:?}", resolve_toggle(toggle, value)),
        Resolver::CeilingDefault => {
            format!("{:?}", default_max_adaptive_concurrency(environment))
        }
    }
}

#[rstest]
#[case::prompts_from_environment(Resolver::PromptsOnly, "null", &[(PROMPTS, "true")], "true")]
#[case::prompts_blank_environment(Resolver::PromptsOnly, "null", &[(PROMPTS, "")], "false")]
#[case::prompts_unset(Resolver::PromptsOnly, "null", &[], "false")]
#[case::prompts_configured_beats_environment(Resolver::PromptsOnly, "false", &[(PROMPTS, "true")], "false")]
#[case::prompts_blank_configured_skips_environment(Resolver::PromptsOnly, "''", &[(PROMPTS, "true")], "false")]
#[case::prompts_invalid_logs_prompts_only(Resolver::PromptsOnly, "null", &[(PROMPTS, "maybe")], "true warned")]
#[case::prompts_whitespace_is_invalid(Resolver::PromptsOnly, "' '", &[], "true warned")]
#[case::prompts_padded_yes(Resolver::PromptsOnly, "' yes '", &[], "true")]
#[case::partition_from_environment(Resolver::Partition, "null", &[(PARTITION, "hour")], "Hour")]
#[case::partition_configured_hour(Resolver::Partition, "hour", &[], "Hour")]
#[case::partition_blank_skips_environment(Resolver::Partition, "''", &[(PARTITION, "hour")], "Day")]
#[case::partition_day(Resolver::Partition, "day", &[], "Day")]
#[case::partition_unset(Resolver::Partition, "null", &[], "Day")]
#[case::partition_wrong_case(Resolver::Partition, "HOUR", &[], "Day warned")]
#[case::partition_padded(Resolver::Partition, "' hour'", &[], "Day warned")]
#[case::partition_not_a_string(Resolver::Partition, "5", &[], "Day warned")]
#[case::uploads_unset(UPLOADS, "null", &[], "7")]
#[case::uploads_blank(UPLOADS, "''", &[], "7")]
#[case::uploads_integer(UPLOADS, "3", &[], "3")]
#[case::uploads_padded_string(UPLOADS, "' 5 '", &[], "5")]
#[case::uploads_plus_sign(UPLOADS, "'+5'", &[], "5")]
#[case::uploads_leading_zero(UPLOADS, "'05'", &[], "5")]
#[case::uploads_underscores(UPLOADS, "'1_000'", &[], "1000")]
#[case::uploads_double_underscore(UPLOADS, "'1__0'", &[], "7 warned")]
#[case::uploads_leading_underscore(UPLOADS, "'_1'", &[], "7 warned")]
#[case::uploads_underscore_after_sign(UPLOADS, "'+_5'", &[], "7 warned")]
#[case::uploads_decimal_zeros(UPLOADS, "'1.000'", &[], "1")]
#[case::uploads_underscores_and_zeros(UPLOADS, "'1_000.0'", &[], "1000")]
#[case::uploads_bare_point(UPLOADS, "'1.'", &[], "7 warned")]
#[case::uploads_point_zero_underscore(UPLOADS, "'1.0_0'", &[], "7 warned")]
#[case::uploads_fraction_string(UPLOADS, "'1.5'", &[], "7 warned")]
#[case::uploads_whole_float(UPLOADS, "4.0", &[], "4")]
#[case::uploads_fraction_float(UPLOADS, "4.5", &[], "7 warned")]
#[case::uploads_hex(UPLOADS, "'0x10'", &[], "7 warned")]
#[case::uploads_exponent(UPLOADS, "'1e3'", &[], "7 warned")]
#[case::uploads_arabic_digit(UPLOADS, "\"\u{663}\"", &[], "7 warned")]
#[case::uploads_python_space(UPLOADS, "\"\\x1c5\"", &[], "5")]
#[case::uploads_negative(UPLOADS, "'-5'", &[], "7 warned")]
#[case::uploads_zero(UPLOADS, "0", &[], "7 warned")]
#[case::uploads_true_is_one(UPLOADS, "true", &[], "1")]
#[case::uploads_false_is_zero(UPLOADS, "false", &[], "7 warned")]
#[case::uploads_list(UPLOADS, "[1]", &[], "7 warned")]
#[case::uploads_beyond_i64(UPLOADS, "100000000000000000000", &[], "7 warned")]
#[case::queue_rejects_true(QUEUE, "true", &[], "7 warned")]
#[case::queue_integer(QUEUE, "12", &[], "12")]
#[case::ceiling_rejects_false(ADAPTIVE_CEILING, "false", &[], "7 warned")]
#[case::ceiling_string(ADAPTIVE_CEILING, "'40'", &[], "40")]
#[case::retry_unset_disables(Resolver::RetryAge, "null", &[], "None")]
#[case::retry_blank_disables(Resolver::RetryAge, "''", &[], "None")]
#[case::retry_zero_disables(Resolver::RetryAge, "0", &[], "None")]
#[case::retry_zero_string_disables(Resolver::RetryAge, "'0'", &[], "None")]
#[case::retry_seconds(Resolver::RetryAge, "5", &[], "Some(5s)")]
#[case::retry_whole_float(Resolver::RetryAge, "2.0", &[], "Some(2s)")]
#[case::retry_rejects_bool(Resolver::RetryAge, "false", &[], "Some(9s) warned")]
#[case::retry_negative(Resolver::RetryAge, "-1", &[], "Some(9s) warned")]
#[case::retry_not_a_number(Resolver::RetryAge, "abc", &[], "Some(9s) warned")]
#[case::drop_unset(DROP, "null", &[], "true")]
#[case::drop_blank(DROP, "''", &[], "true")]
#[case::drop_off(DROP, "'off'", &[], "false")]
#[case::drop_invalid(DROP, "'2'", &[], "true warned")]
#[case::adaptive_unset(ADAPTIVE, "null", &[], "false")]
#[case::adaptive_short_true(ADAPTIVE, "t", &[], "true")]
#[case::adaptive_short_yes(ADAPTIVE, "'Y'", &[], "true")]
#[case::adaptive_mixed_case(ADAPTIVE, "TrUe", &[], "true")]
#[case::adaptive_one(ADAPTIVE, "1", &[], "true")]
#[case::adaptive_zero_float(ADAPTIVE, "0.0", &[], "false")]
#[case::adaptive_one_float(ADAPTIVE, "1.0", &[], "true")]
#[case::adaptive_fraction(ADAPTIVE, "0.5", &[], "false warned")]
#[case::adaptive_two(ADAPTIVE, "2", &[], "false warned")]
#[case::adaptive_float_string(ADAPTIVE, "'1.0'", &[], "false warned")]
#[case::adaptive_mapping(ADAPTIVE, "{}", &[], "false warned")]
#[case::batch_unset(BATCH, "null", &[], "false")]
#[case::batch_on(BATCH, "'on'", &[], "true")]
#[case::batch_short_no(BATCH, "n", &[], "false")]
#[case::batch_list(BATCH, "[]", &[], "false warned")]
#[case::ceiling_default(Resolver::CeilingDefault, "null", &[], "200")]
#[case::ceiling_from_environment(Resolver::CeilingDefault, "null", &[(CEILING, " 64 ")], "64")]
#[case::ceiling_zero(Resolver::CeilingDefault, "null", &[(CEILING, "0")], "0")]
#[case::ceiling_invalid(Resolver::CeilingDefault, "null", &[(CEILING, "abc")], "200")]
#[case::ceiling_decimal(Resolver::CeilingDefault, "null", &[(CEILING, "1.0")], "200")]
#[case::ceiling_negative(Resolver::CeilingDefault, "null", &[(CEILING, "-3")], "200 warned")]
fn s3_option_resolution_behaviors_behavior(
    #[case] resolver: Resolver,
    #[case] configured: &str,
    #[case] variables: &[(&str, &str)],
    #[case] expected: &str,
) {
    let value: Option<OptionValue> = serde_saphyr::from_str(configured).unwrap();
    let environment: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, text)| ((*name).to_owned(), (*text).to_owned()))
        .collect();
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let resolved = with_default(subscriber, || {
        render(resolver, value.as_ref(), &environment)
    });
    let output = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    let shown = if output.contains("WARN") {
        format!("{resolved} warned")
    } else {
        resolved
    };
    assert_eq!(shown, expected, "{output}");
}
