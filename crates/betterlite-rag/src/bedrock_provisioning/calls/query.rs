use betterlite_context::aws_partition::get_aws_partition;
use percent_encoding::utf8_percent_encode;
use quick_xml::Reader;
use quick_xml::escape::unescape;
use quick_xml::events::Event;
use serde::Serialize;

use super::CallRequest;
use super::error::CallError;

pub const GET_CALLER_IDENTITY: &str = "GetCallerIdentity";
pub const CREATE_ROLE: &str = "CreateRole";
pub const PUT_ROLE_POLICY: &str = "PutRolePolicy";

const STS_SERVICE: &str = "sts";
const IAM_SERVICE: &str = "iam";
const FORM_CONTENT_TYPE: &str = "application/x-www-form-urlencoded; charset=utf-8";
const STS_VERSION: &str = "2011-06-15";
const IAM_VERSION: &str = "2010-05-08";

#[derive(Clone, Copy)]
struct IamEndpoint {
    url: &'static str,
    signing_region: &'static str,
}

const IAM_COMMERCIAL: IamEndpoint = IamEndpoint {
    url: "https://iam.amazonaws.com/",
    signing_region: "us-east-1",
};
const IAM_CHINA: IamEndpoint = IamEndpoint {
    url: "https://iam.cn-north-1.amazonaws.com.cn/",
    signing_region: "cn-north-1",
};
const IAM_GOVERNMENT: IamEndpoint = IamEndpoint {
    url: "https://iam.us-gov.amazonaws.com/",
    signing_region: "us-gov-west-1",
};
const IAM_ISO: IamEndpoint = IamEndpoint {
    url: "https://iam.us-iso-east-1.c2s.ic.gov/",
    signing_region: "us-iso-east-1",
};
const IAM_ISO_B: IamEndpoint = IamEndpoint {
    url: "https://iam.us-isob-east-1.sc2s.sgov.gov/",
    signing_region: "us-isob-east-1",
};
const IAM_ISO_E: IamEndpoint = IamEndpoint {
    url: "https://iam.eu-isoe-west-1.cloud.adc-e.uk/",
    signing_region: "eu-isoe-west-1",
};
const IAM_ISO_F: IamEndpoint = IamEndpoint {
    url: "https://iam.us-isof-south-1.csp.hci.ic.gov/",
    signing_region: "us-isof-south-1",
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallerIdentity {
    pub account: String,
    pub arn: String,
}

#[must_use]
pub fn sts_url(region: &str) -> String {
    let suffix = get_aws_partition(Some(region)).dns_suffix;
    format!("https://sts.{region}.{suffix}/")
}

#[must_use]
pub fn caller_identity(region: &str) -> CallRequest {
    CallRequest {
        operation: GET_CALLER_IDENTITY,
        service: STS_SERVICE,
        signing_region: region.to_owned(),
        method: "POST",
        url: sts_url(region),
        headers: form_headers(),
        body: form_body(&[("Action", GET_CALLER_IDENTITY), ("Version", STS_VERSION)]),
    }
}

pub fn caller_identity_from(body: &[u8]) -> Result<CallerIdentity, CallError> {
    let result = "GetCallerIdentityResult";
    Ok(CallerIdentity {
        account: child_text(body, result, "Account", GET_CALLER_IDENTITY)?,
        arn: child_text(body, result, "Arn", GET_CALLER_IDENTITY)?,
    })
}

pub fn create_role<Policy: Serialize>(
    region: &str,
    role_name: &str,
    policy: &Policy,
) -> Result<CallRequest, CallError> {
    let policy_document = super::document(policy, CREATE_ROLE)?;
    Ok(iam_request(
        region,
        CREATE_ROLE,
        &[
            ("Action", CREATE_ROLE),
            ("Version", IAM_VERSION),
            ("RoleName", role_name),
            ("AssumeRolePolicyDocument", policy_document.as_str()),
        ],
    ))
}

pub fn role_arn(body: &[u8], operation: &'static str) -> Result<String, CallError> {
    child_text(body, "Role", "Arn", operation)
}

pub fn put_role_policy<Policy: Serialize>(
    region: &str,
    role_name: &str,
    policy_name: &str,
    policy: &Policy,
) -> Result<CallRequest, CallError> {
    let policy_document = super::document(policy, PUT_ROLE_POLICY)?;
    Ok(iam_request(
        region,
        PUT_ROLE_POLICY,
        &[
            ("Action", PUT_ROLE_POLICY),
            ("Version", IAM_VERSION),
            ("RoleName", role_name),
            ("PolicyName", policy_name),
            ("PolicyDocument", policy_document.as_str()),
        ],
    ))
}

fn iam_endpoint(region: &str) -> IamEndpoint {
    match get_aws_partition(Some(region)).partition {
        "aws-cn" => IAM_CHINA,
        "aws-us-gov" => IAM_GOVERNMENT,
        "aws-iso" => IAM_ISO,
        "aws-iso-b" => IAM_ISO_B,
        "aws-iso-e" => IAM_ISO_E,
        "aws-iso-f" => IAM_ISO_F,
        _ => IAM_COMMERCIAL,
    }
}

fn iam_request(region: &str, operation: &'static str, pairs: &[(&str, &str)]) -> CallRequest {
    let endpoint = iam_endpoint(region);
    CallRequest {
        operation,
        service: IAM_SERVICE,
        signing_region: endpoint.signing_region.to_owned(),
        method: "POST",
        url: endpoint.url.to_owned(),
        headers: form_headers(),
        body: form_body(pairs),
    }
}

fn form_headers() -> Vec<(String, String)> {
    vec![("Content-Type".to_owned(), FORM_CONTENT_TYPE.to_owned())]
}

fn form_body(pairs: &[(&str, &str)]) -> Vec<u8> {
    let mut body = String::new();
    for (index, (name, value)) in pairs.iter().enumerate() {
        if index > 0 {
            body.push('&');
        }
        body.push_str(name);
        body.push('=');
        body.push_str(&utf8_percent_encode(value, super::UNRESERVED).to_string());
    }
    body.into_bytes()
}

// reads the text of the child element inside the named parent, so Role/Arn is not any Arn
fn child_text(
    body: &[u8],
    parent: &str,
    child: &'static str,
    operation: &'static str,
) -> Result<String, CallError> {
    let text = std::str::from_utf8(body).map_err(|_source| CallError::NotUtf8 { operation })?;
    let mut reader = Reader::from_str(text);
    let mut stack: Vec<Vec<u8>> = Vec::new();
    let mut found: Option<String> = None;
    loop {
        let event = reader
            .read_event()
            .map_err(|source| CallError::Xml { operation, source })?;
        match event {
            Event::Start(tag) => stack.push(tag.name().as_ref().to_vec()),
            Event::End(_) => {
                stack.pop();
            }
            Event::Text(value) => {
                if is_child_of(&stack, parent, child) {
                    let decoded = value
                        .xml10_content()
                        .map_err(|source| CallError::Decode { operation, source })?;
                    let unescaped = unescape(decoded.as_ref())
                        .map_err(|source| CallError::Escape { operation, source })?;
                    found = Some(unescaped.into_owned());
                }
            }
            Event::CData(value) => {
                if is_child_of(&stack, parent, child) {
                    let decoded = value
                        .xml10_content()
                        .map_err(|source| CallError::Decode { operation, source })?;
                    found = Some(decoded.into_owned());
                }
            }
            Event::Eof => break,
            Event::Empty(_)
            | Event::GeneralRef(_)
            | Event::Decl(_)
            | Event::PI(_)
            | Event::DocType(_)
            | Event::Comment(_) => {}
        }
    }
    found.ok_or(CallError::MissingField {
        operation,
        field: child,
    })
}

fn is_child_of(stack: &[Vec<u8>], parent: &str, child: &str) -> bool {
    let mut names = stack.iter().rev();
    let last = names.next();
    let before = names.next();
    last.is_some_and(|name| name.as_slice() == child.as_bytes())
        && before.is_some_and(|name| name.as_slice() == parent.as_bytes())
}

mod tests;
