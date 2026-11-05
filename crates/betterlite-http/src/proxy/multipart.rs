use crate::proxy::transport_error::TransportError;

#[derive(Debug, Clone)]
pub struct MultipartPart {
    pub name: String,
    pub file_name: Option<String>,
    pub content_type: Option<String>,
    pub data: Vec<u8>,
}

fn quoted(name: &str) -> Result<&str, TransportError> {
    if name.contains(['"', '\r', '\n']) {
        return Err(TransportError::MultipartName {
            name: name.to_owned(),
        });
    }
    Ok(name)
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
        let file_name = match &part.file_name {
            Some(file_name) => format!("; filename=\"{}\"", quoted(file_name)?),
            None => String::new(),
        };
        let disposition = format!(
            "Content-Disposition: form-data; name=\"{}\"{file_name}",
            quoted(&part.name)?
        );
        body.extend_from_slice(format!("{disposition}\r\n").as_bytes());
        if let Some(content_type) = &part.content_type {
            body.extend_from_slice(
                format!("Content-Type: {}\r\n", quoted(content_type)?).as_bytes(),
            );
        }
        body.extend_from_slice(b"\r\n");
        body.extend_from_slice(&part.data);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("{delimiter}--\r\n").as_bytes());
    Ok(body)
}
