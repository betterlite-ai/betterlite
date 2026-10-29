use serde_json::Value;

use crate::mock::ProviderRequest;
use crate::multipart_oracle::Part;
use crate::multipart_oracle::capture::decode_multipart_capture;

pub(crate) fn captured_parts(request: &ProviderRequest) -> Option<Vec<Part>> {
    let content_type = request.headers.get("content-type")?;
    let Value::Array(values) = &request.body else {
        return None;
    };
    let bytes = values
        .iter()
        .map(|value| u8::try_from(value.as_u64()?).ok())
        .collect::<Option<Vec<u8>>>()?;
    decode_multipart_capture(content_type, &bytes).ok()
}

pub(crate) fn first_part_difference(left: &[Part], right: &[Part]) -> Option<String> {
    let count = left.len().max(right.len());
    (0..count).find_map(|index| {
        let field = match (left.get(index), right.get(index)) {
            (Some(left_part), Some(right_part)) if left_part.name != right_part.name => "/name",
            (Some(left_part), Some(right_part)) if left_part.filename != right_part.filename => {
                "/filename"
            }
            (Some(left_part), Some(right_part))
                if left_part.content_type != right_part.content_type =>
            {
                "/content_type"
            }
            (Some(left_part), Some(right_part)) if left_part.body != right_part.body => "/body",
            (Some(_), Some(_)) => return None,
            (Some(_), None) | (None, _) => "",
        };
        Some(format!("/parts/{index}{field}"))
    })
}

pub(crate) fn masked_boundary(value: &str) -> String {
    let Some((head, tail)) = value
        .starts_with("multipart/")
        .then(|| value.split_once("boundary="))
        .flatten()
    else {
        return value.to_owned();
    };
    let rest = tail.find(';').and_then(|end| tail.get(end..)).unwrap_or("");
    format!("{head}boundary=<boundary>{rest}")
}
