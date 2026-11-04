#![cfg(test)]
use betterlite_auth::auth_identity::records::Principal;
use rust_decimal::Decimal;
use serde_json::json;

use betterlite_transcribe::policy::{
    job_access_refusal, requested_media_format, started_transcription_job,
    transcribe_admin_only_refusal, transcribe_cost_per_second, transcribe_max_job_cost,
    transcribe_media_buckets, transcribe_owned_start_request, transcribe_storage_refusal,
    transcription_job_cost, unpriceable_request_reason,
};

#[test]
fn cost_per_second_reads_model_rate_or_returns_none() {
    assert_eq!(
        transcribe_cost_per_second(
            &json!({"transcribe/StartTranscriptionJob":{"input_cost_per_second":"0.002"}})
        ),
        Some(Decimal::new(2, 3))
    );
    assert_eq!(transcribe_cost_per_second(&json!({})), None);
}

#[test]
fn unpriceable_request_reason_checks_rate_surcharge_and_media_format() {
    assert!(
        unpriceable_request_reason("StartTranscriptionJob", &json!({"MediaFormat":"wav"}), None)
            .is_some()
    );
    assert!(
        unpriceable_request_reason(
            "StartTranscriptionJob",
            &json!({"MediaFormat":"wav","ToxicityDetection":{}}),
            Some(Decimal::ONE)
        )
        .is_some()
    );
    assert!(
        unpriceable_request_reason(
            "StartTranscriptionJob",
            &json!({"MediaFormat":"aac"}),
            Some(Decimal::ONE)
        )
        .is_some()
    );
    assert!(
        unpriceable_request_reason("StartMedicalScribeJob", &json!({}), Some(Decimal::ONE))
            .is_some()
    );
}

#[test]
fn requested_media_format_prefers_field_then_lowercases_uri_extension() {
    assert_eq!(
        requested_media_format(
            &json!({"MediaFormat":"WAV","Media":{"MediaFileUri":"s3://bucket/file.mp3"}})
        ),
        Some("wav".to_owned())
    );
    assert_eq!(
        requested_media_format(&json!({"Media":{"MediaFileUri":"s3://bucket/file.MP3"}})),
        Some("mp3".to_owned())
    );
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
}

#[test]
fn job_access_refusal_hides_foreign_jobs_as_not_found() {
    let principal: Principal = serde_json::from_value(json!({"principal_type":"human","subject":"user-1","auth_method":"api_key","user":{"id":"user-1"}})).unwrap();
    let foreign_job =
        json!({"TranscriptionJob":{"Tags":[{"Key":"litellm-owner","Value":"other"}]}});
    assert_eq!(
        job_access_refusal(&foreign_job, Some(&principal)).map(|error| error.status_code),
        Some(404)
    );
    assert_eq!(job_access_refusal(&foreign_job, None), None);
}

#[test]
fn started_transcription_job_rejects_malformed_response() {
    assert!(started_transcription_job(&json!({"TranscriptionJob":{"Tags":"bad"}})).is_none());
}

#[test]
fn admin_only_refusal_allows_job_operations_and_blocks_account_wide_operations() {
    let principal: Principal = serde_json::from_value(json!({"principal_type":"human","subject":"user-1","auth_method":"api_key","user":{"id":"user-1"}})).unwrap();
    assert_eq!(
        transcribe_admin_only_refusal("StartTranscriptionJob", Some(&principal)),
        None
    );
    assert_eq!(
        transcribe_admin_only_refusal("GetTranscriptionJob", Some(&principal)),
        None
    );
    assert_eq!(
        transcribe_admin_only_refusal("CreateVocabulary", Some(&principal))
            .map(|refusal| refusal.status_code),
        Some(403)
    );
}

#[test]
fn media_buckets_parses_list_and_storage_refusal_checks_buckets_roles_and_admin() {
    let principal: Principal = serde_json::from_value(json!({"principal_type":"human","subject":"user-1","auth_method":"api_key","user":{"id":"user-1"}})).unwrap();
    assert!(transcribe_media_buckets(&json!({"transcribe_media_buckets":"audio"})).is_none());
    let settings = json!({"transcribe_media_buckets":["audio"]});
    let buckets = transcribe_media_buckets(&settings).unwrap();
    assert!(
        transcribe_storage_refusal(
            &json!({"Media":{"MediaFileUri":"s3://foreign/a.wav"}}),
            Some(&buckets),
            Some(&principal)
        )
        .is_some()
    );
    assert!(
        transcribe_storage_refusal(
            &json!({"DataAccessRoleArn":"arn"}),
            Some(&buckets),
            Some(&principal)
        )
        .is_some()
    );
    assert!(
        transcribe_storage_refusal(
            &json!({"Media":{"MediaFileUri":"s3://foreign/a.wav"}}),
            None,
            Some(&principal)
        )
        .is_some()
    );
}

#[test]
fn owned_start_request_adds_primary_owner_tag_and_rejects_invalid_tags() {
    let principal: Principal = serde_json::from_value(json!({"principal_type":"human","subject":"user-1","auth_method":"api_key","user":{"id":"user-1"}})).unwrap();
    let owned = transcribe_owned_start_request(&json!({"Tags":[]}), Some(&principal)).unwrap();
    assert_eq!(owned["Tags"][0]["Key"], "litellm-owner");
    assert_eq!(owned["Tags"][0]["Value"], "user-1");
    assert_eq!(
        transcribe_owned_start_request(
            &json!({"Tags":[{"Key":"litellm-owner","Value":"other"}]}),
            Some(&principal)
        )
        .err()
        .map(|refusal| refusal.status_code),
        Some(400)
    );
    assert_eq!(
        transcribe_owned_start_request(&json!({}), None)
            .err()
            .map(|refusal| refusal.status_code),
        Some(400)
    );
}

struct TestJobLookup {
    fail: bool,
}

impl betterlite_transcribe::records::JobLookup for TestJobLookup {
    fn get_transcription_job<'a>(
        &'a self,
        _request: &'a betterlite_transcribe::records::GetTranscriptionJobRequest,
    ) -> betterlite_transcribe::records::JobLookupFuture<'a> {
        Box::pin(async move {
            if self.fail {
                return Err(betterlite_transcribe::records::JobLookupError::Unavailable);
            }
            Ok(serde_json::from_value(
                json!({"TranscriptionJob":{"Tags":[{"Key":"litellm-owner","Value":"foreign"}]}}),
            )
            .unwrap())
        })
    }
}

#[tokio::test]
async fn transcribe_job_access_refusal_returns_404_for_lookup_failure_and_foreign_job() {
    let principal: Principal = serde_json::from_value(json!({"principal_type":"human","subject":"user-1","auth_method":"api_key","user":{"id":"user-1"}})).unwrap();
    let job_name = json!("job-1");
    let failed_lookup = TestJobLookup { fail: true };
    assert_eq!(
        betterlite_transcribe::policy::transcribe_job_access_refusal(
            &job_name,
            Some(&principal),
            &failed_lookup
        )
        .await
        .map(|refusal| refusal.status_code),
        Some(404)
    );
    let foreign_lookup = TestJobLookup { fail: false };
    assert_eq!(
        betterlite_transcribe::policy::transcribe_job_access_refusal(
            &job_name,
            Some(&principal),
            &foreign_lookup
        )
        .await
        .map(|refusal| refusal.status_code),
        Some(404)
    );
}

#[test]
fn transcription_cost_overflow_returns_typed_error() {
    assert_eq!(
        transcription_job_cost(Decimal::MAX, Decimal::MAX),
        Err(betterlite_transcribe::policy::TranscribeCostError::Overflow)
    );
}
