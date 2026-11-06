pub const US_REGIONS: [&str; 6] = [
    "us-east-1",
    "us-east-2",
    "us-west-1",
    "us-west-2",
    "us-gov-east-1",
    "us-gov-west-1",
];

pub const EU_REGIONS: [&str; 8] = [
    "eu-west-1",
    "eu-west-2",
    "eu-west-3",
    "eu-central-1",
    "eu-central-2",
    "eu-south-1",
    "eu-south-2",
    "eu-north-1",
];

pub const AP_REGIONS: [&str; 7] = [
    "ap-northeast-1",
    "ap-northeast-2",
    "ap-northeast-3",
    "ap-south-1",
    "ap-south-2",
    "ap-southeast-1",
    "ap-southeast-2",
];

pub const CA_REGIONS: [&str; 1] = ["ca-central-1"];

pub const SA_REGIONS: [&str; 1] = ["sa-east-1"];

pub const CROSS_REGION_PREFIXES: [&str; 7] = ["global", "us", "eu", "apac", "jp", "au", "us-gov"];

#[derive(Clone, Copy, Debug, Default)]
pub struct AmazonBedrockGlobalConfig;

impl AmazonBedrockGlobalConfig {
    #[must_use]
    pub fn get_all_regions(&self) -> Vec<&'static str> {
        let mut regions = Vec::new();
        regions.extend_from_slice(&US_REGIONS);
        regions.extend_from_slice(&EU_REGIONS);
        regions.extend_from_slice(&AP_REGIONS);
        regions.extend_from_slice(&CA_REGIONS);
        regions.extend_from_slice(&SA_REGIONS);
        regions
    }
}
