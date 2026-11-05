use crate::proxy::transport_error::TransportError;

#[derive(Debug, Clone)]
pub struct MultipartPart {
    pub name: String,
    pub file_name: Option<String>,
    pub content_type: Option<String>,
    pub data: Vec<u8>,
}

// the HTML5 form-encoding rules httpx applies to a field name or file name
fn escaped(text: &str) -> String {
    text.chars()
        .map(|letter| match letter {
            '"' => "%22".to_owned(),
            '\\' => "\\\\".to_owned(),
            '\u{0}'..='\u{1a}' | '\u{1c}'..='\u{1f}' => format!("%{:02X}", u32::from(letter)),
            other => other.to_string(),
        })
        .collect()
}

pub fn encode(boundary: &str, parts: &[MultipartPart]) -> Result<Vec<u8>, TransportError> {
    let delimiter = format!("--{boundary}");
    let mut body = Vec::new();
    for part in parts {
        if part
            .data
            .windows(delimiter.len())
            .any(|window| window == delimiter.as_bytes())
        {
            return Err(TransportError::MultipartBoundary {
                boundary: boundary.to_owned(),
            });
        }
        body.extend_from_slice(format!("{delimiter}\r\n").as_bytes());
        let file_name = part
            .file_name
            .as_ref()
            .map_or_else(String::new, |file_name| {
                format!("; filename=\"{}\"", escaped(file_name))
            });
        let disposition = format!(
            "Content-Disposition: form-data; name=\"{}\"{file_name}",
            escaped(&part.name)
        );
        body.extend_from_slice(format!("{disposition}\r\n").as_bytes());
        if let Some(content_type) = &part.content_type {
            if content_type.contains(['\r', '\n']) {
                return Err(TransportError::MultipartContentType {
                    content_type: content_type.clone(),
                });
            }
            body.extend_from_slice(format!("Content-Type: {content_type}\r\n").as_bytes());
        }
        body.extend_from_slice(b"\r\n");
        body.extend_from_slice(&part.data);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("{delimiter}--\r\n").as_bytes());
    Ok(body)
}
