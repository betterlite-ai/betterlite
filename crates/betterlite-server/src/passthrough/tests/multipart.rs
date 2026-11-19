use std::time::Duration;

use betterlite_core::api::request_form::values::{FormPart, PartValue, UploadFile};
use betterlite_http::proxy::multipart::MultipartPart;
use http::Method;
use http::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use url::Url;

use crate::passthrough::upstream::UpstreamTarget;

mod passthrough_build_request_files;
mod passthrough_is_multipart;
mod passthrough_make_multipart;

fn field(name: &str, value: &str) -> FormPart {
    FormPart {
        name: name.to_owned(),
        value: PartValue::Text(value.to_owned()),
    }
}

fn upload(name: &str, filename: &str, content_type: Option<&str>, content: &[u8]) -> FormPart {
    let uploaded = UploadFile {
        filename: filename.to_owned(),
        content_type: content_type.map(str::to_owned),
        content: content.to_vec(),
        size: Some(u64::try_from(content.len()).unwrap()),
    };
    FormPart {
        name: name.to_owned(),
        value: PartValue::Upload(uploaded),
    }
}

fn target(headers: HeaderMap) -> UpstreamTarget {
    UpstreamTarget {
        method: Method::POST,
        url: Url::parse("https://upstream.test/v1/files").unwrap(),
        headers,
        query: vec![("page".to_owned(), "2".to_owned())],
        deadline: Duration::from_secs(5),
    }
}

fn headers_with(content_type: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_str(content_type).unwrap());
    headers.insert(
        HeaderName::from_static("x-custom"),
        HeaderValue::from_static("kept"),
    );
    headers
}

#[derive(Debug, PartialEq, Eq)]
struct SentPart {
    name: String,
    file_name: Option<String>,
    content_type: Option<String>,
    data: Vec<u8>,
}

fn rows_of(parts: Vec<MultipartPart>) -> Vec<SentPart> {
    parts
        .into_iter()
        .map(|part| SentPart {
            name: part.name,
            file_name: part.file_name,
            content_type: part.content_type,
            data: part.data,
        })
        .collect()
}

fn row(name: &str, part_type: Option<(&str, &str)>, data: &[u8]) -> SentPart {
    let (file_name, content_type) = part_type.unzip();
    SentPart {
        name: name.to_owned(),
        file_name: file_name.map(str::to_owned),
        content_type: content_type.map(str::to_owned),
        data: data.to_vec(),
    }
}

fn header_text(headers: &HeaderMap, name: &'static str) -> Option<String> {
    headers
        .get(HeaderName::from_static(name))
        .map(|value| value.to_str().unwrap().to_owned())
}
