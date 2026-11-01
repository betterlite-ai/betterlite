use std::str::Utf8Error;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum FormCaptureError {
    #[error("form response is not valid UTF-8: {0}")]
    Utf8(#[source] Utf8Error),
    #[error("response has {count} form elements, not exactly one")]
    FormCount { count: usize },
    #[error("cannot parse selector \"{0}\"")]
    Selector(String),
    #[error("cannot parse response as HTML: {0}")]
    Html(#[source] tl::ParseError),
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FormProjection {
    pub method: String,
    pub action: Option<String>,
    pub controls: Vec<(String, Option<String>)>,
}

pub(crate) fn parse_form_projection(text: &str) -> Result<FormProjection, FormCaptureError> {
    let dom = tl::parse(text, tl::ParserOptions::default()).map_err(FormCaptureError::Html)?;
    let parser = dom.parser();
    let forms = dom
        .query_selector("form")
        .ok_or_else(|| FormCaptureError::Selector("form".to_owned()))?
        .collect::<Vec<_>>();
    let [form] = forms.as_slice() else {
        return Err(FormCaptureError::FormCount { count: forms.len() });
    };
    let form_tag = form
        .get(parser)
        .and_then(tl::Node::as_tag)
        .ok_or(FormCaptureError::FormCount { count: 0 })?;
    let method = form_tag.attributes().get("method").flatten().map_or_else(
        || "get".to_owned(),
        |value| value.as_utf8_str().to_ascii_lowercase(),
    );
    let action = form_tag
        .attributes()
        .get("action")
        .flatten()
        .map(|value| value.as_utf8_str().into_owned());
    let controls = form_tag
        .query_selector(parser, "input[name], button[name]")
        .ok_or_else(|| FormCaptureError::Selector("input[name], button[name]".to_owned()))?
        .map(|handle| control_pair(handle, parser))
        .collect();
    Ok(FormProjection {
        method,
        action,
        controls,
    })
}

fn control_pair(handle: tl::NodeHandle, parser: &tl::Parser<'_>) -> (String, Option<String>) {
    let tag = handle.get(parser).and_then(tl::Node::as_tag);
    let name = tag
        .and_then(|element| element.attributes().get("name").flatten())
        .map_or_else(String::new, |value| value.as_utf8_str().into_owned());
    let value = tag
        .and_then(|element| element.attributes().get("value").flatten())
        .map(|value| value.as_utf8_str().into_owned());
    (name, value)
}
