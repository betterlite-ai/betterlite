use betterlite_context::aws_partition::get_aws_partition;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

// Python's quote leaves the unreserved characters alone, so only these and "/" stay as themselves
const OBJECT_KEY_SAFE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~')
    .remove(b'/');

#[must_use]
pub(super) fn object_url(bucket_name: &str, region: &str, object_key: &str) -> String {
    let key = utf8_percent_encode(object_key, OBJECT_KEY_SAFE);
    let dns_suffix = get_aws_partition(Some(region)).dns_suffix;
    if is_virtual_hosted(bucket_name) {
        return format!("https://{bucket_name}.s3.{region}.{dns_suffix}/{key}");
    }
    format!("https://s3.{region}.{dns_suffix}/{bucket_name}/{key}")
}

// boto3 asks for a path-style URL unless the bucket name passes its virtual-host DNS check
fn is_virtual_hosted(bucket_name: &str) -> bool {
    let bytes = bucket_name.as_bytes();
    let alphanumeric = |byte: &u8| byte.is_ascii_lowercase() || byte.is_ascii_digit();
    (3..=63).contains(&bytes.len())
        && bytes.first().is_some_and(alphanumeric)
        && bytes.last().is_some_and(alphanumeric)
        && bytes.iter().all(|byte| alphanumeric(byte) || *byte == b'-')
}
