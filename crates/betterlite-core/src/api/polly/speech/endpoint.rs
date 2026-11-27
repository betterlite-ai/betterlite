use betterlite_context::aws_partition::get_aws_partition;

// LiteLLM's Polly config defaults the URL and signing region to us-east-1 when none is given
pub const DEFAULT_REGION: &str = "us-east-1";
const SPEECH_PATH: &str = "/v1/speech";

#[must_use]
pub fn complete_url(api_base: Option<&str>, region: Option<&str>) -> String {
    if let Some(base) = api_base {
        return format!("{}{SPEECH_PATH}", base.trim_end_matches('/'));
    }
    let selected = region.unwrap_or(DEFAULT_REGION);
    let suffix = get_aws_partition(Some(selected)).dns_suffix;
    format!("https://polly.{selected}.{suffix}{SPEECH_PATH}")
}
