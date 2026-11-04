#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AwsPartition {
    pub partition: &'static str,
    pub dns_suffix: &'static str,
}

#[path = "aws_partition/tests.rs"]
mod tests;

const COMMERCIAL: AwsPartition = AwsPartition {
    partition: "aws",
    dns_suffix: "amazonaws.com",
};

pub fn get_aws_partition(region: Option<&str>) -> AwsPartition {
    let Some(region) = region.filter(|region| !region.is_empty()) else {
        return COMMERCIAL;
    };
    let (partition, dns_suffix) = if region.starts_with("cn-") {
        ("aws-cn", "amazonaws.com.cn")
    } else if region.starts_with("us-gov-") {
        ("aws-us-gov", "amazonaws.com")
    } else if region.starts_with("us-isob-") {
        ("aws-iso-b", "sc2s.sgov.gov")
    } else if region.starts_with("us-isof-") {
        ("aws-iso-f", "csp.hci.ic.gov")
    } else if region.starts_with("us-iso-") {
        ("aws-iso", "c2s.ic.gov")
    } else if region.starts_with("eu-isoe-") {
        ("aws-iso-e", "cloud.adc-e.uk")
    } else {
        return COMMERCIAL;
    };
    AwsPartition {
        partition,
        dns_suffix,
    }
}

pub fn contains_bedrock_arn(value: &str) -> bool {
    value.match_indices("arn:aws").any(|(index, _)| {
        value
            .get(index..)
            .is_some_and(|remaining| is_bedrock_arn(remaining))
    })
}
pub fn is_bedrock_arn(value: &str) -> bool {
    match value.strip_prefix("arn:aws") {
        Some(tail) if tail.starts_with(":bedrock:") => true,
        Some(tail) if tail.starts_with('-') => {
            tail.split_once(':').is_some_and(|(partition, service)| {
                valid_partition(partition) && service.starts_with("bedrock:")
            })
        }
        _ => false,
    }
}
pub fn contains_aws_arn(value: &str) -> bool {
    value
        .split("arn:")
        .skip(1)
        .any(|suffix| match suffix.strip_prefix("aws") {
            Some(":") | Some(":s3") => true,
            Some(rest) if rest.starts_with(':') => true,
            Some(rest) if rest.starts_with('-') => rest
                .split_once(':')
                .is_some_and(|(partition, _)| valid_partition(partition)),
            _ => false,
        })
}

fn valid_partition(value: &str) -> bool {
    value.is_empty()
        || value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}
