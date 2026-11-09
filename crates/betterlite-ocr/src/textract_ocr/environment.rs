use std::sync::Arc;

use secrecy::ExposeSecret as _;

use crate::json::Json;
use crate::ocr_auth::{AwsRequestSigner, AwsSigningContext};
use crate::ocr_config::{OcrEnvironment, PreparedOcrRequest};
use crate::ocr_error::OcrError;
use crate::ocr_http::OcrClient;
use crate::textract_ocr::TextractOperation;

const TEXTRACT_SERVICE: &str = "textract";
const AWS_JSON_CONTENT_TYPE: &str = "application/x-amz-json-1.1";
const TARGET_HEADER: &str = "X-Amz-Target";
const CONTENT_TYPE_HEADER: &str = "Content-Type";
const MISSING_REGION: &str =
    "the AWS region is missing: pass aws_region_name or set AWS_REGION_NAME or AWS_REGION";

pub(crate) const AWS_SECRET_NAMES: [&str; 11] = [
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "AWS_REGION_NAME",
    "AWS_REGION",
    "AWS_SESSION_NAME",
    "AWS_PROFILE_NAME",
    "AWS_ROLE_NAME",
    "AWS_WEB_IDENTITY_TOKEN",
    "AWS_STS_ENDPOINT",
    "AWS_EXTERNAL_ID",
];

pub struct TextractEnvironment {
    headers: Vec<(String, String)>,
    region: String,
    signer: Arc<dyn AwsRequestSigner>,
}

impl OcrEnvironment for TextractEnvironment {
    fn headers(&self) -> &[(String, String)] {
        &self.headers
    }

    fn signer(&self) -> Option<AwsSigningContext<'_>> {
        Some(AwsSigningContext {
            signer: self.signer.as_ref(),
            region: &self.region,
            service: TEXTRACT_SERVICE,
        })
    }
}

pub(crate) fn textract_environment(
    request: &PreparedOcrRequest,
    operation: TextractOperation,
    client: &OcrClient,
) -> Result<TextractEnvironment, OcrError> {
    let named_region = match request.optional_params.get("aws_region_name") {
        Some(Json::Text(region)) => Some(region.clone()),
        Some(_) | None => None,
    };
    let region = named_region
        .or_else(|| {
            request
                .connection
                .secrets
                .get("AWS_REGION_NAME")
                .map(|value| value.expose_secret().to_owned())
        })
        .or_else(|| {
            request
                .connection
                .secrets
                .get("AWS_REGION")
                .map(|value| value.expose_secret().to_owned())
        })
        .ok_or_else(|| OcrError::InvalidRequest(MISSING_REGION.to_owned()))?;
    Ok(TextractEnvironment {
        headers: operation_headers(&request.connection.extra_headers, operation),
        region,
        signer: Arc::clone(&client.aws),
    })
}

// A caller's copy of an operation header would reach the wire next to ours while the signature covers only one value
pub(crate) fn operation_headers(
    extra_headers: &[(String, String)],
    operation: TextractOperation,
) -> Vec<(String, String)> {
    let required = [
        (TARGET_HEADER, operation.target()),
        (CONTENT_TYPE_HEADER, AWS_JSON_CONTENT_TYPE),
    ];
    extra_headers
        .iter()
        .filter(|(name, _)| {
            !required
                .iter()
                .any(|(operation_name, _)| name.eq_ignore_ascii_case(operation_name))
        })
        .cloned()
        .chain(
            required
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned())),
        )
        .collect()
}

pub(crate) fn endpoint(request: &PreparedOcrRequest, environment: &TextractEnvironment) -> String {
    let region = environment.region.as_str();
    request
        .connection
        .api_base
        .clone()
        .unwrap_or_else(|| format!("https://textract.{region}.amazonaws.com/"))
}
