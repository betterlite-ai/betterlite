#![cfg(test)]
use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::Role;
use rstest::{fixture, rstest};
use rust_decimal::Decimal;

use crate::field::Field;
use crate::policy::cost::{
    TranscribeCostError, started_transcription_job, transcribe_cost_per_second,
    transcribe_max_job_cost, transcription_job_cost,
};
use crate::policy::eligibility::{
    requested_media_format, transcribe_admin_only_refusal, unpriceable_request_reason,
};
use crate::policy::ownership::{
    job_access_refusal, transcribe_job_access_refusal, transcribe_owned_start_request,
};
use crate::policy::storage::{transcribe_media_buckets, transcribe_storage_refusal};
use crate::records::{GetTranscriptionJobRequest, JobLookup, JobLookupError, JobLookupFuture};
use crate::start_request::OwnedStartRequest;

#[fixture]
fn principal() -> Principal {
    serde_json::from_str(r#"{"principal_type":"human","subject":"user-1","auth_method":"api_key","user":{"id":"user-1"}}"#).unwrap()
}

#[rstest]
#[case::number(
    r#"{"transcribe/StartTranscriptionJob":{"input_cost_per_second":0.002}}"#,
    Some(Decimal::new(2, 3))
)]
#[case::string(
    r#"{"transcribe/StartTranscriptionJob":{"input_cost_per_second":"0.002"}}"#,
    Some(Decimal::new(2, 3))
)]
#[case::absent("{}", None)]
#[case::malformed(
    r#"{"transcribe/StartTranscriptionJob":{"input_cost_per_second":true}}"#,
    None
)]
fn cost_per_second_reads_model_rate_or_returns_none(
    #[case] input: &str,
    #[case] expected: Option<Decimal>,
) {
    assert_eq!(transcribe_cost_per_second(input), expected);
}

#[rstest]
#[case::rate(
    "StartTranscriptionJob",
    r#"{"MediaFormat":"wav"}"#,
    None,
    "input_cost_per_second"
)]
#[case::toxicity(
    "StartTranscriptionJob",
    r#"{"MediaFormat":"wav","ToxicityDetection":[]}"#,
    Some(Decimal::ONE),
    "ToxicityDetection"
)]
#[case::redaction(
    "StartTranscriptionJob",
    r#"{"MediaFormat":"wav","ContentRedaction":null}"#,
    Some(Decimal::ONE),
    "ContentRedaction"
)]
#[case::custom_model(
    "StartTranscriptionJob",
    r#"{"MediaFormat":"wav","ModelSettings":{"LanguageModelName":"custom"}}"#,
    Some(Decimal::ONE),
    "ModelSettings.LanguageModelName"
)]
#[case::language_model(
    "StartTranscriptionJob",
    r#"{"MediaFormat":"wav","LanguageIdSettings":{"en-US":{"LanguageModelName":"custom"}}}"#,
    Some(Decimal::ONE),
    "LanguageIdSettings.en-US.LanguageModelName"
)]
#[case::format(
    "StartTranscriptionJob",
    r#"{"MediaFormat":"aac"}"#,
    Some(Decimal::ONE),
    "Media format"
)]
#[case::medical(
    "StartMedicalScribeJob",
    "{}",
    Some(Decimal::ONE),
    "StartMedicalScribeJob"
)]
#[case::call_analytics(
    "StartCallAnalyticsJob",
    "{}",
    Some(Decimal::ONE),
    "StartCallAnalyticsJob"
)]
#[case::medical_transcription(
    "StartMedicalTranscriptionJob",
    "{}",
    Some(Decimal::ONE),
    "StartMedicalTranscriptionJob"
)]
fn unpriceable_request_reason_checks_rate_surcharge_and_media_format(
    #[case] operation: &str,
    #[case] input: &str,
    #[case] rate: Option<Decimal>,
    #[case] reason: &str,
) {
    assert!(
        unpriceable_request_reason(operation, input, rate)
            .unwrap()
            .contains(reason)
    );
}

#[rstest]
#[case::priced("StartTranscriptionJob", r#"{"MediaFormat":"wav"}"#)]
#[case::unbilled("GetTranscriptionJob", "{}")]
fn unpriceable_request_reason_allows_supported_requests(
    #[case] operation: &str,
    #[case] input: &str,
) {
    assert_eq!(
        unpriceable_request_reason(operation, input, Some(Decimal::ONE)),
        None
    );
}

#[rstest]
#[case::explicit(
    r#"{"MediaFormat":"WAV","Media":{"MediaFileUri":"s3://bucket/file.mp3"}}"#,
    Some("wav")
)]
#[case::extension(r#"{"Media":{"MediaFileUri":"s3://bucket/file.MP3"}}"#, Some("mp3"))]
#[case::query(
    r#"{"Media":{"MediaFileUri":"https://s3.example/bucket/file.WAV?q=.mp3"}}"#,
    Some("wav")
)]
#[case::empty_explicit(
    r#"{"MediaFormat":"","Media":{"MediaFileUri":"s3://bucket/file.wav"}}"#,
    Some("")
)]
#[case::missing("{}", None)]
fn requested_media_format_prefers_field_then_lowercases_uri_extension(
    #[case] input: &str,
    #[case] expected: Option<&str>,
) {
    assert_eq!(requested_media_format(input).as_deref(), expected);
}

#[test]
fn rounds_seconds_for_transcription_job_cost() {
    assert_eq!(
        transcription_job_cost(Decimal::new(101, 2), Decimal::new(2, 3)),
        Ok(Decimal::new(4, 3))
    );
    assert_eq!(
        transcribe_max_job_cost(Decimal::new(2, 3)),
        Ok(Decimal::new(72, 1))
    );
    assert_eq!(
        transcription_job_cost(Decimal::MAX, Decimal::MAX),
        Err(TranscribeCostError::Overflow)
    );
    assert_eq!(
        transcribe_max_job_cost(Decimal::MAX),
        Err(TranscribeCostError::Overflow)
    );
}

#[rstest]
#[case::foreign(
    r#"{"TranscriptionJob":{"Tags":[{"Key":"litellm-owner","Value":"other"}]}}"#,
    Some(404)
)]
#[case::absent(r#"{"TranscriptionJob":{}}"#, Some(404))]
#[case::malformed(r#"{"TranscriptionJob":{"Tags":"bad"}}"#, Some(404))]
#[case::owned(
    r#"{"TranscriptionJob":{"Tags":[{"Key":"litellm-owner","Value":"user-1"}]}}"#,
    None
)]
#[case::first_owner(r#"{"TranscriptionJob":{"Tags":[{"Key":"litellm-owner","Value":"other"},{"Key":"litellm-owner","Value":"user-1"}]}}"#, Some(404))]
fn job_access_refusal_hides_foreign_jobs_as_not_found(
    principal: Principal,
    #[case] input: &str,
    #[case] expected: Option<u16>,
) {
    assert_eq!(
        job_access_refusal(input, Some(&principal)).map(|error| error.status_code),
        expected
    );
}

#[rstest]
#[case::bad_tags(r#"{"TranscriptionJob":{"Tags":"bad"}}"#)]
#[case::absent("{}")]
#[case::invalid_json("{")]
fn started_transcription_job_rejects_malformed_response(#[case] input: &str) {
    assert!(started_transcription_job(input).is_none());
}

#[test]
fn started_transcription_job_reads_record_defaults() {
    let job = started_transcription_job(r#"{"TranscriptionJob":{"TranscriptionJobStatus":"COMPLETED","CreationTime":123,"Media":{"MediaFileUri":"s3://audio/a.wav"}}}"#).unwrap();
    assert_eq!(job.transcription_job_status.as_deref(), Some("COMPLETED"));
    assert_eq!(
        job.media.unwrap().media_file_uri.as_deref(),
        Some("s3://audio/a.wav")
    );
    assert!(job.tags.is_empty());
}

#[rstest]
#[case::start("StartTranscriptionJob", None)]
#[case::get("GetTranscriptionJob", None)]
#[case::delete("DeleteTranscriptionJob", None)]
#[case::account_wide("CreateVocabulary", Some(403))]
fn admin_only_refusal_allows_job_operations_and_blocks_account_wide_operations(
    principal: Principal,
    #[case] operation: &str,
    #[case] expected: Option<u16>,
) {
    assert_eq!(
        transcribe_admin_only_refusal(operation, Some(&principal))
            .map(|refusal| refusal.status_code),
        expected
    );
}

#[rstest]
#[case::malformed(r#"{"transcribe_media_buckets":"audio"}"#)]
#[case::null(r#"{"transcribe_media_buckets":null}"#)]
#[case::missing("{}")]
fn media_buckets_rejects_invalid_config(#[case] input: &str) {
    assert!(transcribe_media_buckets(input).is_none());
}

#[rstest]
#[case::media(r#"{"Media":{"MediaFileUri":"s3://foreign/a.wav"}}"#)]
#[case::redacted(r#"{"Media":{"RedactedMediaFileUri":"s3://foreign/a.wav"}}"#)]
#[case::output(r#"{"OutputBucketName":"foreign"}"#)]
#[case::https(r#"{"Media":{"MediaFileUri":"https://audio.s3.amazonaws.com/a.wav"}}"#)]
#[case::null_media(r#"{"Media":{"MediaFileUri":null}}"#)]
#[case::role(r#"{"DataAccessRoleArn":"arn"}"#)]
#[case::settings(r#"{"JobExecutionSettings":{}}"#)]
#[case::null_role(r#"{"DataAccessRoleArn":null}"#)]
fn media_buckets_parses_list_and_storage_refusal_checks_buckets_roles_and_admin(
    principal: Principal,
    #[case] input: &str,
) {
    let buckets =
        transcribe_media_buckets(r#"{"transcribe_media_buckets":["audio","audio"]}"#).unwrap();
    assert_eq!(buckets.len(), 1);
    assert_eq!(
        transcribe_storage_refusal(input, Some(&buckets), Some(&principal))
            .unwrap()
            .status_code,
        403
    );
    assert_eq!(
        transcribe_storage_refusal("{}", None, Some(&principal))
            .unwrap()
            .status_code,
        403
    );
    assert_eq!(
        transcribe_storage_refusal(
            r#"{"Media":{"MediaFileUri":"s3://audio/a.wav","RedactedMediaFileUri":"s3://audio/redacted.wav"},"OutputBucketName":"audio"}"#,
            Some(&buckets),
            Some(&principal)
        ),
        None
    );
}

#[rstest]
#[case::malformed(r#"{"Tags":"bad"}"#)]
#[case::non_object(r#"{"Tags":[1]}"#)]
#[case::missing_key(r#"{"Tags":[{}]}"#)]
#[case::empty_key(r#"{"Tags":[{"Key":""}]}"#)]
#[case::bad_value(r#"{"Tags":[{"Key":"x","Value":1}]}"#)]
#[case::forged(r#"{"Tags":[{"Key":"litellm-owner","Value":"other"}]}"#)]
fn owned_start_request_rejects_invalid_tags(principal: Principal, #[case] input: &str) {
    assert_eq!(
        transcribe_owned_start_request(input, Some(&principal))
            .unwrap_err()
            .status_code,
        400
    );
}

#[rstest]
fn owned_start_request_adds_primary_owner_tag_and_rejects_invalid_tags(principal: Principal) {
    let owned = transcribe_owned_start_request(r#"{"Tags":[]}"#, Some(&principal)).unwrap();
    let owner = owned.tags.first().unwrap();
    assert_eq!(owner.key, Field::Present(Some("litellm-owner".to_owned())));
    assert_eq!(owner.value, Field::Present(Some("user-1".to_owned())));
    assert_eq!(
        transcribe_owned_start_request("{}", None)
            .unwrap_err()
            .status_code,
        400
    );
}

#[rstest]
#[case::values(r#"{"LanguageCode":"en-US","Settings":{"ChannelIdentification":true},"Tags":[{"Key":"team","Value":"research","FutureField":true}]}"#)]
#[case::null_value(r#"{"Tags":[{"Key":"team","Value":null}]}"#)]
#[case::absent_value(r#"{"Tags":[{"Key":"team"}]}"#)]
#[case::null_key(r#"{"Tags":[{"Key":null}]}"#)]
fn owned_start_request_preserves_passthrough_fields(principal: Principal, #[case] input: &str) {
    let before: OwnedStartRequest = serde_json::from_str(input).unwrap();
    let output = transcribe_owned_start_request(input, Some(&principal)).unwrap();
    let encoded = serde_json::to_string(&output).unwrap();
    let decoded: OwnedStartRequest = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded.passthrough, before.passthrough);
    assert_eq!(decoded.tags.first(), before.tags.first());
    assert_eq!(decoded.tags.len(), 2);
    assert_eq!(before.tags.len(), 1);
}

struct TestJobLookup {
    fail: bool,
}

impl JobLookup for TestJobLookup {
    fn get_transcription_job<'lookup>(
        &'lookup self,
        request: &'lookup GetTranscriptionJobRequest,
    ) -> JobLookupFuture<'lookup> {
        Box::pin(async move {
            assert_eq!(request.transcription_job_name, "job-1");
            if self.fail {
                return Err(JobLookupError::Unavailable);
            }
            Ok(serde_json::from_str(
                r#"{"TranscriptionJob":{"Tags":[{"Key":"litellm-owner","Value":"foreign"}]}}"#,
            )
            .unwrap())
        })
    }
}

#[rstest]
#[case::lookup_failure(true)]
#[case::foreign(false)]
#[tokio::test]
async fn transcribe_job_access_refusal_returns_404_for_lookup_failure_and_foreign_job(
    principal: Principal,
    #[case] fail: bool,
) {
    let lookup = TestJobLookup { fail };
    assert_eq!(
        transcribe_job_access_refusal(r#""job-1""#, Some(&principal), &lookup)
            .await
            .unwrap()
            .status_code,
        404
    );
    assert_eq!(
        transcribe_job_access_refusal("42", Some(&principal), &lookup)
            .await
            .unwrap()
            .status_code,
        400
    );
}

#[rstest]
#[tokio::test]
async fn admin_only_refusal_storage_refusal_and_job_access_refusal_bypass_admin(
    mut principal: Principal,
) {
    principal.roles.push(Role::PlatformAdmin);
    assert_eq!(
        transcribe_admin_only_refusal("CreateVocabulary", Some(&principal)),
        None
    );
    assert_eq!(
        transcribe_storage_refusal("invalid", None, Some(&principal)),
        None
    );
    assert_eq!(job_access_refusal("invalid", Some(&principal)), None);
    assert_eq!(
        transcribe_job_access_refusal("invalid", Some(&principal), &TestJobLookup { fail: true })
            .await,
        None
    );
}

#[test]
fn transcription_cost_overflow_returns_typed_error() {
    assert_eq!(
        transcription_job_cost(Decimal::MAX, Decimal::MAX),
        Err(TranscribeCostError::Overflow)
    );
}

#[rstest]
#[case::missing("{}")]
#[case::null(r#"{"Tags":null}"#)]
fn owned_start_request_defaults_tags(principal: Principal, #[case] input: &str) {
    let owned = transcribe_owned_start_request(input, Some(&principal)).unwrap();
    assert_eq!(owned.tags.len(), 1);
}
