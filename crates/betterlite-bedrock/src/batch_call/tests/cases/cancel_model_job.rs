use std::collections::BTreeMap;

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_openai::batches::records::BatchStatus;
use secrecy::ExposeSecret as _;

use crate::batch_call::cancel::{CancelBatchCall, cancel_batch};
use crate::batch_call::transport::header;

use super::super::fixture::{
    ACCESS_KEY, CONFIGURED_ACCESS_KEY, CONFIGURED_SECRET_KEY, CONFIGURED_SESSION_TOKEN,
    ENCODED_JOB_ARN, Factory, JOB_ARN, JOB_ID, REGION, SESSION_TOKEN, Source, answer,
    auth_parameters, configured_auth, credentials, now, parameters, runtime,
};

const STOPPED: &str = r#"{"jobArn":"arn:aws:bedrock:eu-west-1:123456789012:model-invocation-job/job-1","status":"Stopped"}"#;

pub(super) fn cancel_model_job() {
    explicit_region_wins();
    arn_names_the_region();
    missing_region_defaults();
}

fn explicit_region_wins() {
    let source = Source::new(credentials());
    let factory = Factory::new(vec![answer(200, ""), answer(200, STOPPED)]);
    let kwargs = parameters(&configured_auth());
    let call = CancelBatchCall {
        batch_id: JOB_ID,
        aws_region_name: Some("ap-southeast-2"),
        kwargs: &kwargs,
        now: now(),
    };
    let batch = runtime()
        .block_on(cancel_batch(call, &source, &factory, None))
        .expect("the stop and the lookup succeed");
    assert_eq!(batch.id, JOB_ARN);
    assert_eq!(batch.status, BatchStatus::Cancelled);
    assert_eq!(batch.input_file_id, "");
    assert_stop_requests(&factory);
    assert_configured_auth(&source);
}

// the returned batch is the control plane's record, so the stop body itself is parsed and dropped
fn assert_stop_requests(factory: &Factory) {
    let requests = factory.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(
        requests[0].url,
        "https://bedrock.ap-southeast-2.amazonaws.com/model-invocation-job/job-1/stop"
    );
    assert_eq!(requests[1].method, "GET");
    assert_eq!(
        requests[1].url,
        "https://bedrock.ap-southeast-2.amazonaws.com/model-invocation-job/job-1"
    );
    assert!(requests[0].body.is_empty());
    assert_eq!(
        header(&requests[0].headers, "x-amz-security-token"),
        Some(SESSION_TOKEN)
    );
    assert_signed_with(&requests[0].headers, ACCESS_KEY, "ap-southeast-2");
    let configs = factory.configs();
    assert_eq!(configs.len(), 2);
    assert_eq!(configs[0].service_name, "bedrock");
    assert_eq!(configs[0].signature_version, "v4");
    assert_eq!(configs[0].access_key_id, ACCESS_KEY);
}

fn assert_signed_with(headers: &[(String, String)], access: &str, region: &str) {
    let authorization = header(headers, "authorization").expect("the request is signed");
    assert!(authorization.contains(&format!("Credential={access}/")));
    assert!(authorization.contains(&format!("/{region}/bedrock/aws4_request")));
}

fn assert_configured_auth(source: &Source) {
    let settings = source.settings();
    assert_eq!(settings.len(), 2);
    let auth: &AwsSettings = &settings[0];
    assert_eq!(auth.region_name.as_deref(), Some("ap-southeast-2"));
    assert_eq!(
        auth.access_key_id
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        Some(CONFIGURED_ACCESS_KEY)
    );
    assert_eq!(
        auth.secret_access_key
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        Some(CONFIGURED_SECRET_KEY)
    );
    assert_eq!(
        auth.session_token
            .as_ref()
            .map(|token| token.expose_secret().as_str()),
        Some(CONFIGURED_SESSION_TOKEN)
    );
    assert_eq!(auth.session_name.as_deref(), Some("session"));
    assert_eq!(auth.profile_name.as_deref(), Some("profile"));
    assert_eq!(auth.role_name.as_deref(), Some("role"));
    assert_eq!(auth.sts_endpoint.as_deref(), Some("https://sts.example"));
    assert_eq!(auth.external_id.as_deref(), Some("external"));
    assert!(auth.web_identity_token.is_some());
    assert!(auth.session_tags.is_some());
    assert_eq!(source.regions(), ["ap-southeast-2", "ap-southeast-2"]);
}

fn arn_names_the_region() {
    let source = Source::new(credentials());
    let factory = Factory::new(vec![answer(200, ""), answer(200, STOPPED)]);
    let kwargs = auth_parameters();
    let call = CancelBatchCall {
        batch_id: JOB_ARN,
        aws_region_name: None,
        kwargs: &kwargs,
        now: now(),
    };
    let batch = runtime()
        .block_on(cancel_batch(call, &source, &factory, None))
        .expect("the stop and the lookup succeed");
    assert_eq!(batch.status, BatchStatus::Cancelled);
    let requests = factory.requests();
    assert_eq!(
        requests[0].url,
        format!(
            "https://bedrock.{REGION}.amazonaws.com/model-invocation-job/{ENCODED_JOB_ARN}/stop"
        )
    );
    assert_eq!(
        requests[1].url,
        format!("https://bedrock.{REGION}.amazonaws.com/model-invocation-job/{ENCODED_JOB_ARN}")
    );
    assert_eq!(source.regions(), [REGION, REGION]);
}

fn missing_region_defaults() {
    let source = Source::new(credentials());
    let factory = Factory::new(vec![answer(200, ""), answer(200, STOPPED)]);
    let kwargs = BTreeMap::new();
    let call = CancelBatchCall {
        batch_id: JOB_ID,
        aws_region_name: None,
        kwargs: &kwargs,
        now: now(),
    };
    let batch = runtime()
        .block_on(cancel_batch(call, &source, &factory, None))
        .expect("the stop and the lookup succeed");
    assert_eq!(batch.status, BatchStatus::Cancelled);
    let requests = factory.requests();
    assert_eq!(
        requests[0].url,
        "https://bedrock.us-east-1.amazonaws.com/model-invocation-job/job-1/stop"
    );
    assert_eq!(
        requests[1].url,
        "https://bedrock.us-east-1.amazonaws.com/model-invocation-job/job-1"
    );
    assert_eq!(source.regions(), ["us-east-1", "us-east-1"]);
}
