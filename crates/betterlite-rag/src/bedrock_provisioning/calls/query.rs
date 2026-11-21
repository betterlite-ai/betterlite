use std::str;

use betterlite_context::aws_partition::get_aws_partition;
use percent_encoding::utf8_percent_encode;
use quick_xml::Reader;
use quick_xml::escape::unescape;
use quick_xml::events::{BytesCData, BytesText, Event};

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
        headers: vec![("Content-Type".to_owned(), FORM_CONTENT_TYPE.to_owned())],
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

// the IAM query protocol carries every field of the operation in the form body
#[must_use]
pub fn iam_request(region: &str, operation: &'static str, pairs: &[(&str, &str)]) -> CallRequest {
    let endpoint = iam_endpoint(region);
    let mut fields: Vec<(&str, &str)> = vec![("Action", operation), ("Version", IAM_VERSION)];
    fields.extend_from_slice(pairs);
    CallRequest {
        operation,
        service: IAM_SERVICE,
        signing_region: endpoint.signing_region.to_owned(),
        method: "POST",
        url: endpoint.url.to_owned(),
        headers: vec![("Content-Type".to_owned(), FORM_CONTENT_TYPE.to_owned())],
        body: form_body(&fields),
    }
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

// reads the named child's text; quick-xml emits an entity as its own event, so the pieces are appended
pub fn child_text(
    body: &[u8],
    parent: &str,
    child: &'static str,
    operation: &'static str,
) -> Result<String, CallError> {
    let text = str::from_utf8(body).map_err(|_source| CallError::NotUtf8 { operation })?;
    find_child_text(text, parent, child, operation)?.ok_or(CallError::MissingField {
        operation,
        field: child,
    })
}

fn find_child_text(
    text: &str,
    parent: &str,
    child: &str,
    operation: &'static str,
) -> Result<Option<String>, CallError> {
    let mut reader = Reader::from_str(text);
    let mut stack: Vec<Vec<u8>> = Vec::new();
    let mut found: Option<String> = None;
    loop {
        let event = reader
            .read_event()
            .map_err(|source| CallError::Xml { operation, source })?;
        match event {
            Event::Start(tag) => {
                stack.push(tag.name().as_ref().to_vec());
                if is_child_of(&stack, parent, child) {
                    found = Some(String::new());
                }
            }
            Event::End(_) => {
                stack.pop();
            }
            Event::Text(value) => {
                if is_child_of(&stack, parent, child) {
                    let piece = text_value(&value, operation)?;
                    found.get_or_insert_with(String::new).push_str(&piece);
                }
            }
            Event::CData(value) => {
                if is_child_of(&stack, parent, child) {
                    let piece = cdata_value(&value, operation)?;
                    found.get_or_insert_with(String::new).push_str(&piece);
                }
            }
            Event::GeneralRef(reference) => {
                if is_child_of(&stack, parent, child) {
                    let decoded = reference
                        .decode()
                        .map_err(|source| CallError::Decode { operation, source })?;
                    let raw = format!("&{decoded};");
                    let piece =
                        unescape(&raw).map_err(|source| CallError::Escape { operation, source })?;
                    found.get_or_insert_with(String::new).push_str(&piece);
                }
            }
            Event::Eof => break,
            Event::Empty(_)
            | Event::Decl(_)
            | Event::PI(_)
            | Event::DocType(_)
            | Event::Comment(_) => {}
        }
    }
    Ok(found)
}

fn text_value(value: &BytesText<'_>, operation: &'static str) -> Result<String, CallError> {
    let decoded = value
        .xml10_content()
        .map_err(|source| CallError::Decode { operation, source })?;
    let unescaped =
        unescape(decoded.as_ref()).map_err(|source| CallError::Escape { operation, source })?;
    Ok(unescaped.into_owned())
}

fn cdata_value(value: &BytesCData<'_>, operation: &'static str) -> Result<String, CallError> {
    let decoded = value
        .xml10_content()
        .map_err(|source| CallError::Decode { operation, source })?;
    Ok(decoded.into_owned())
}

fn is_child_of(stack: &[Vec<u8>], parent: &str, child: &str) -> bool {
    let mut names = stack.iter().rev();
    let last = names.next();
    let before = names.next();
    last.is_some_and(|name| name.as_slice() == child.as_bytes())
        && before.is_some_and(|name| name.as_slice() == parent.as_bytes())
}

mod tests;
