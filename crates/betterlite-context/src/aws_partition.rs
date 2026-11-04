#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AwsPartition {
    pub partition: &'static str,
    pub dns_suffix: &'static str,
    pub arn_prefix: &'static str,
}

mod tests;

const COMMERCIAL: AwsPartition = AwsPartition {
    partition: "aws",
    dns_suffix: "amazonaws.com",
    arn_prefix: "arn:aws:",
};

#[must_use]
pub fn get_aws_partition(region_name: Option<&str>) -> AwsPartition {
    let Some(region) = region_name.filter(|name| !name.is_empty()) else {
        return COMMERCIAL;
    };
    let (partition, dns_suffix, arn_prefix) = if region.starts_with("cn-") {
        ("aws-cn", "amazonaws.com.cn", "arn:aws-cn:")
    } else if region.starts_with("us-gov-") {
        ("aws-us-gov", "amazonaws.com", "arn:aws-us-gov:")
    } else if region.starts_with("us-isob-") {
        ("aws-iso-b", "sc2s.sgov.gov", "arn:aws-iso-b:")
    } else if region.starts_with("us-isof-") {
        ("aws-iso-f", "csp.hci.ic.gov", "arn:aws-iso-f:")
    } else if region.starts_with("us-iso-") {
        ("aws-iso", "c2s.ic.gov", "arn:aws-iso:")
    } else if region.starts_with("eu-isoe-") {
        ("aws-iso-e", "cloud.adc-e.uk", "arn:aws-iso-e:")
    } else {
        return COMMERCIAL;
    };
    AwsPartition {
        partition,
        dns_suffix,
        arn_prefix,
    }
}

#[must_use]
pub fn contains_bedrock_arn(value: &str) -> bool {
    value
        .match_indices("arn:")
        .filter_map(|(offset, prefix)| value.get(offset..)?.strip_prefix(prefix))
        .any(|candidate| {
            aws_service(candidate).is_some_and(|service| service.starts_with("bedrock"))
        })
}

#[must_use]
pub fn is_bedrock_arn(value: &str) -> bool {
    value
        .strip_prefix("arn:")
        .and_then(aws_service)
        .is_some_and(|service| service.starts_with("bedrock:"))
}

#[must_use]
pub fn contains_aws_arn(value: &str) -> bool {
    value
        .match_indices("arn:")
        .filter_map(|(offset, prefix)| value.get(offset..)?.strip_prefix(prefix))
        .any(|candidate| aws_service(candidate).is_some())
}

fn aws_service(candidate: &str) -> Option<&str> {
    let tail = candidate.strip_prefix("aws")?;
    if let Some(service) = tail.strip_prefix(':') {
        return Some(service);
    }
    let (partition, service) = tail.strip_prefix('-')?.split_once(':')?;
    let valid = !partition.is_empty()
        && partition
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    valid.then_some(service)
}
