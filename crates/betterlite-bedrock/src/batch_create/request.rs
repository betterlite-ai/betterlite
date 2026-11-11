pub mod build;
pub mod sign;
pub mod wire;

const ROLE_ARN_PARAM: &str = "aws_batch_role_arn";
const ROLE_ARN_ENV: &str = "AWS_BATCH_ROLE_ARN";
const OUTPUT_BUCKET_PARAM: &str = "s3_output_bucket_name";
const OUTPUT_BUCKET_ENV: &str = "AWS_S3_OUTPUT_BUCKET_NAME";
const OWNER_PARAM: &str = "s3_bucket_owner";
const ENCRYPTION_KEY_PARAM: &str = "s3_encryption_key_id";
const TAGS_PARAM: &str = "bedrock_tags";
const DEFAULT_PREFIX: &str = "litellm";
const JOB_ID_LENGTH: usize = 8;
const COMPLETION_WINDOW_24H: &str = "24h";
const TIMEOUT_HOURS: i64 = 24;
