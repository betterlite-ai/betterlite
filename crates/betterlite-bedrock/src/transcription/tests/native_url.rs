use super::support::{env, params};
use crate::transcription::url::{bedrock_model_id_and_region, complete_url};

type Row = (
    &'static str,
    Option<&'static str>,
    &'static str,
    &'static [(&'static str, &'static str)],
    &'static str,
);

const ARN: &str = "arn:aws:bedrock:eu-central-1:123:foundation-model/x";
const OPEN: &str = r#"{"aws_bedrock_runtime_endpoint":" https://rt.example/ "}"#;
const API: Option<&str> = Some("https://api.example");

// "@region|model" stands for the regional runtime endpoint of that model
const URL_ROWS: [Row; 24] = [
    ("bedrock/m.v1", None, "{}", &[], "@us-west-2|m.v1"),
    ("bedrock/us-east-1/m.v1", None, "{}", &[], "@us-east-1|m.v1"),
    ("converse/eu-west-1/nova/m", None, "{}", &[], "@eu-west-1|m"),
    ("bedrock/converse/nova-2/m", None, "{}", &[], "@nova-2|m"),
    (
        "bedrock/converse/us-east-1/nova-2/m",
        None,
        "{}",
        &[],
        "@us-east-1|m",
    ),
    ("bedrock/nova/m", None, "{}", &[], "@us-west-2|m"),
    ("bedrock/abc/m", None, "{}", &[], "@us-west-2|abc/m"),
    ("bedrock/a-b/m", None, "{}", &[], "@us-west-2|a-b/m"),
    ("bedrock/us-e/m", None, "{}", &[], "@us-e|m"),
    ("bedrock/us_e-1/m", None, "{}", &[], "@us-west-2|us_e-1/m"),
    (
        ARN,
        None,
        "{}",
        &[],
        "@eu-central-1|arn:aws:bedrock:eu-central-1:123:foundation-model/x",
    ),
    (
        "arn:aws:bedrock::1:m",
        None,
        "{}",
        &[],
        "@us-west-2|arn:aws:bedrock::1:m",
    ),
    (
        "bedrock/us-east-1/m",
        None,
        r#"{"aws_region_name":"ap-south-1"}"#,
        &[],
        "@ap-south-1|m",
    ),
    (
        "bedrock/us-east-1/m",
        None,
        r#"{"aws_region_name":""}"#,
        &[],
        "@|m",
    ),
    (
        "m",
        None,
        "{}",
        &[("AWS_REGION_NAME", "r1"), ("AWS_REGION", "r2")],
        "@r1|m",
    ),
    ("m", None, "{}", &[("AWS_REGION", "r2")], "@r2|m"),
    (
        "m",
        None,
        "{}",
        &[("AWS_REGION_NAME", ""), ("AWS_REGION", "r2")],
        "@|m",
    ),
    (
        "m",
        None,
        "{}",
        &[("AWS_DEFAULT_REGION", "r3")],
        "@us-west-2|m",
    ),
    (
        "m",
        Some("https://api.example///"),
        "{}",
        &[],
        "https://api.example/model/m/converse",
    ),
    ("m", API, OPEN, &[], "https://rt.example/model/m/converse"),
    (
        "m",
        API,
        r#"{"aws_bedrock_runtime_endpoint":""}"#,
        &[],
        "@us-west-2|m",
    ),
    (
        "m",
        API,
        r#"{"aws_bedrock_runtime_endpoint":5}"#,
        &[],
        "https://api.example/model/m/converse",
    ),
    ("m", Some("  "), "{}", &[], "@us-west-2|m"),
    (
        "m",
        None,
        "{}",
        &[("AWS_BEDROCK_RUNTIME_ENDPOINT", "https://e")],
        "@us-west-2|m",
    ),
];

fn expected_url(shorthand: &str) -> String {
    match shorthand
        .strip_prefix('@')
        .and_then(|rest| rest.split_once('|'))
    {
        Some((region, model)) => {
            format!("https://bedrock-runtime.{region}.amazonaws.com/model/{model}/converse")
        }
        None => shorthand.to_owned(),
    }
}

pub(super) fn native_transcription_url() {
    for (model, api_base, source, variables, expected) in URL_ROWS {
        let url = complete_url(api_base, model, &params(source), &env(variables));
        assert_eq!(url, expected_url(expected), "{model} {source}");
    }
    let (model, region) = bedrock_model_id_and_region("bedrock/us-east-1/m");
    assert_eq!((model, region), ("m", Some("us-east-1")));
    let unicode = complete_url(None, "a b/ü", &params("{}"), &env(&[]));
    assert_eq!(unicode, expected_url("@us-west-2|a b/ü"));
}
