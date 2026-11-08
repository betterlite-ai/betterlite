use crate::oci_common::OciError;

pub const OCI_API_VERSION: &str = "20231130";
pub const OCI_REGION_PATTERN: &str = "^[a-z][a-z0-9-]{0,30}[a-z0-9]$";

pub fn get_oci_base_url(region: &str, api_base: Option<&str>) -> Result<String, OciError> {
    if let Some(base) = api_base.filter(|value| !value.is_empty()) {
        return Ok(strip_action_path(base).trim_end_matches('/').to_owned());
    }
    if !valid_region(region) {
        return Err(OciError {
            status: 400,
            message: format!(
                "Invalid OCI region '{region}': must match {OCI_REGION_PATTERN} (e.g. 'us-ashburn-1')."
            ),
        });
    }
    Ok(format!(
        "https://inference.generativeai.{region}.oci.oraclecloud.com"
    ))
}

fn strip_action_path(base: &str) -> &str {
    let marker = format!("/{OCI_API_VERSION}/actions/");
    let without_slash = base.strip_suffix('/').unwrap_or(base);
    let Some(index) = without_slash.rfind(marker.as_str()) else {
        return base;
    };
    let Some(name_start) = index.checked_add(marker.len()) else {
        return base;
    };
    let Some(name) = without_slash.get(name_start..) else {
        return base;
    };
    if name.is_empty() || name.contains(['/', '?', '#']) {
        return base;
    }
    base.get(..index).unwrap_or(base)
}

fn valid_region(region: &str) -> bool {
    let bytes = region.as_bytes();
    if !(2..=32).contains(&bytes.len()) {
        return false;
    }
    let Some((&first, rest)) = bytes.split_first() else {
        return false;
    };
    let Some((&last, middle)) = rest.split_last() else {
        return false;
    };
    first.is_ascii_lowercase()
        && (last.is_ascii_lowercase() || last.is_ascii_digit())
        && middle
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}
