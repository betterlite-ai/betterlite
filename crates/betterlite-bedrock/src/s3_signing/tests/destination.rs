use std::collections::BTreeMap;

use betterlite_openai::files::records::CreatePurpose;
use betterlite_prompts::media_metadata::{FileBody, FileData};

use super::support::{ID, params};
use crate::request_policy::{Params, RequestValue};
use crate::s3_signing::destination::{UploadTarget, get_complete_file_url};
use crate::signing::target::RegionInputs;

const OBJECT: &str = "litellm-bedrock-files/11111111222243338444555555555555-a_b.txt";

struct Inputs {
    litellm_params: Params,
    optional_params: Params,
    file: FileData,
    environment: BTreeMap<String, String>,
}

fn inputs(litellm: &[(&str, &str)], optional: &[(&str, &str)]) -> Inputs {
    Inputs {
        litellm_params: params(litellm),
        optional_params: params(optional),
        file: FileData::Tuple2 {
            filename: Some("a b.txt".to_owned()),
            content: FileBody::Bytes(b"hello".to_vec()),
        },
        environment: BTreeMap::new(),
    }
}

fn target(inputs: &Inputs) -> UploadTarget<'_> {
    UploadTarget {
        model: "bedrock/anthropic.claude",
        litellm_params: &inputs.litellm_params,
        optional_params: &inputs.optional_params,
        file: Some(&inputs.file),
        purpose: Some(CreatePurpose::Assistants),
        environment: &inputs.environment,
        region_inputs: RegionInputs::default(),
        id: ID,
    }
}

fn url(target: &UploadTarget<'_>) -> String {
    match get_complete_file_url(target) {
        Ok(url) => url,
        Err(error) => format!("error: {error}"),
    }
}

fn located(litellm: &[(&str, &str)], optional: &[(&str, &str)]) -> String {
    url(&target(&inputs(litellm, optional)))
}

pub(super) fn upload_destination() {
    bucket_and_prefix();
    region_precedence();
    endpoint_choice();
    failure_order();
}

fn bucket_and_prefix() {
    let expected = format!("https://s3.us-west-2.amazonaws.com/bkt/{OBJECT}");
    assert_eq!(located(&[("s3_bucket_name", "bkt")], &[]), expected);
    let prefixed = format!("https://s3.us-west-2.amazonaws.com/bkt/pre/fix/{OBJECT}");
    assert_eq!(
        located(&[("s3_bucket_name", " bkt/pre/fix/ ")], &[]),
        prefixed
    );
    // an empty configured bucket falls back to AWS_S3_BUCKET_NAME
    let mut from_env = inputs(&[("s3_bucket_name", "")], &[]);
    from_env
        .environment
        .insert("AWS_S3_BUCKET_NAME".to_owned(), "env-bkt".to_owned());
    assert_eq!(
        url(&target(&from_env)),
        format!("https://s3.us-west-2.amazonaws.com/env-bkt/{OBJECT}")
    );
    from_env
        .environment
        .insert("AWS_S3_BUCKET_NAME".to_owned(), String::new());
    let missing = "error: S3 bucket_name is required. Set 's3_bucket_name' in litellm_params or AWS_S3_BUCKET_NAME env var";
    assert_eq!(url(&target(&from_env)), missing);
    assert_eq!(
        located(&[], &[("s3_bucket_name", "optional-only")]),
        missing
    );
    // batch content names the object after its first record's model
    let mut batch = inputs(&[("s3_bucket_name", "bkt")], &[]);
    batch.file = FileData::Direct(FileBody::Bytes(b"{\"body\": {\"model\": \"m\"}}".to_vec()));
    let batch_target = UploadTarget {
        purpose: Some(CreatePurpose::Batch),
        ..target(&batch)
    };
    let batch_name = "litellm-bedrock-files-m-11111111-2222-4333-8444-555555555555.jsonl";
    assert_eq!(
        url(&batch_target),
        format!("https://s3.us-west-2.amazonaws.com/bkt/{batch_name}")
    );
}

fn region_precedence() {
    let bucket = ("s3_bucket_name", "bkt");
    let at = |region: &str| format!("https://s3.{region}.amazonaws.com/bkt/{OBJECT}");
    assert_eq!(
        located(
            &[bucket, ("s3_region_name", "eu-west-1")],
            &[("s3_region_name", "ap-south-1")]
        ),
        at("eu-west-1")
    );
    assert_eq!(
        located(
            &[bucket, ("s3_region_name", "")],
            &[("s3_region_name", "ap-south-1")]
        ),
        at("ap-south-1")
    );
    // s3_region_name is used as given, while the aws_region_name fallback is validated
    assert_eq!(
        located(
            &[bucket, ("s3_region_name", "Weird")],
            &[("aws_region_name", "Bad")]
        ),
        at("Weird")
    );
    assert_eq!(
        located(&[bucket], &[("aws_region_name", "us-east-2")]),
        at("us-east-2")
    );
    assert_eq!(
        located(&[bucket], &[("aws_region_name", "Bad")]),
        "error: invalid AWS region format: Bad"
    );
    let mut arn = inputs(&[bucket], &[]);
    arn.litellm_params.insert(
        "s3_region_name".to_owned(),
        RequestValue::Raw("null".to_owned()),
    );
    let arn_target = UploadTarget {
        model: "arn:aws:bedrock:eu-central-1:123456789012:inference-profile/x",
        ..target(&arn)
    };
    assert_eq!(url(&arn_target), at("eu-central-1"));
    let from_env = UploadTarget {
        region_inputs: RegionInputs {
            environment_region_name: Some("ca-central-1"),
            environment_region: None,
            session_region: None,
        },
        ..target(&arn)
    };
    assert_eq!(url(&from_env), at("ca-central-1"));
    let china = located(&[bucket, ("s3_region_name", "cn-north-1")], &[]);
    assert_eq!(
        china,
        format!("https://s3.cn-north-1.amazonaws.com.cn/bkt/{OBJECT}")
    );
}

fn endpoint_choice() {
    let bucket = ("s3_bucket_name", "bkt");
    let custom = format!("http://minio:9000/bkt/{OBJECT}");
    assert_eq!(
        located(&[bucket, ("s3_endpoint_url", "http://minio:9000///")], &[]),
        custom
    );
    assert_eq!(
        located(&[bucket], &[("s3_endpoint_url", "http://minio:9000")]),
        custom
    );
    let deployment_wins = located(
        &[bucket, ("s3_endpoint_url", "http://minio:9000")],
        &[("s3_endpoint_url", "http://other")],
    );
    assert_eq!(deployment_wins, custom);
    assert_eq!(
        located(&[bucket, ("s3_endpoint_url", "")], &[]),
        format!("https://s3.us-west-2.amazonaws.com/bkt/{OBJECT}")
    );
}

fn failure_order() {
    let bucket = ("s3_bucket_name", "bkt");
    // the bucket is checked before the region, the region before the file, and the file before the purpose
    assert!(
        located(&[], &[("aws_region_name", "Bad")])
            .starts_with("error: S3 bucket_name is required")
    );
    let no_file = inputs(&[bucket], &[("aws_region_name", "Bad")]);
    let missing_everything = UploadTarget {
        file: None,
        purpose: None,
        ..target(&no_file)
    };
    assert_eq!(
        url(&missing_everything),
        "error: invalid AWS region format: Bad"
    );
    let valid = inputs(&[bucket], &[]);
    assert_eq!(
        url(&UploadTarget {
            file: None,
            purpose: None,
            ..target(&valid)
        }),
        "error: file is required"
    );
    assert_eq!(
        url(&UploadTarget {
            purpose: None,
            ..target(&valid)
        }),
        "error: purpose is required"
    );
    let mut bare = inputs(&[bucket], &[]);
    bare.file = FileData::Direct(FileBody::BareString("/etc/passwd".to_owned()));
    assert!(url(&target(&bare)).starts_with("error: file content is a bare string"));
    assert_eq!(
        located(&[("s3_bucket_name", " / ")], &[]),
        "error: Cloud storage bucket name is required"
    );
}
