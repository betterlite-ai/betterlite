#![cfg(test)]

use super::{contains_aws_arn, contains_bedrock_arn, is_bedrock_arn};
use rstest::rstest;

#[rstest]
#[case::none("none", [false, false, false])]
#[case::commercial("arn:aws:bedrock:r", [true, true, true])]
#[case::partition("arn:aws-iso-b:bedrock:r", [true, true, true])]
#[case::arn_in_partition("arn:aws-barn:bedrock:x", [true, true, true])]
#[case::arn_in_partition_s3("arn:aws-barn:s3:x", [false, false, true])]
#[case::embedded("x arn:aws-cn:bedrock:r", [true, false, true])]
#[case::runtime("arn:aws:bedrock-runtime:r", [true, false, true])]
#[case::agent("arn:aws:bedrock-agent:r", [true, false, true])]
#[case::no_trailing_colon("arn:aws:bedrock", [true, false, true])]
#[case::s3("arn:aws:s3:r", [false, false, true])]
#[case::empty_partition("arn:aws-:bedrock:r", [false, false, false])]
#[case::invalid_partition("arn:aws-!:bedrock:r", [false, false, false])]
#[case::uppercase_partition("arn:aws-CN:bedrock:r", [false, false, false])]
#[case::digit_partition("arn:aws-a9-:bedrock:r", [true, true, true])]
#[case::hyphen_partition("arn:aws--:bedrock:r", [true, true, true])]
#[case::missing_colon("arn:aws", [false, false, false])]
#[case::prefix_collision("arn:awsx:bedrock:r", [false, false, false])]
#[case::later_match("arn:aws-!:bedrock arn:aws:bedrock:r", [true, false, true])]
#[case::unicode("\u{e9} arn:aws-cn:bedrock:r", [true, false, true])]
fn call_context_tests_behavior(#[case] value: &str, #[case] expected: [bool; 3]) {
    assert_eq!(
        [
            contains_bedrock_arn(value),
            is_bedrock_arn(value),
            contains_aws_arn(value)
        ],
        expected
    );
}
