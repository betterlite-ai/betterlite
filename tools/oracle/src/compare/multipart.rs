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

fn generated_filenames_match(left: &str, right: &str) -> bool {
    let (Some((left_stem, left_extension)), Some((right_stem, right_extension))) =
        (left.rsplit_once('.'), right.rsplit_once('.'))
    else {
        return false;
    };
    if left_extension != right_extension || left_stem.len() != right_stem.len() {
        return false;
    }
    // Python spells one generated name as 32 hex characters and the other as 36 hyphenated ones
    [left_stem, right_stem]
        .into_iter()
        .all(|stem| uuid::Uuid::try_parse(stem).is_ok_and(|uuid| uuid.get_version_num() == 4))
}

// a selected part lets its two generated names differ, so the walk may compare everything else
pub(crate) fn mask_generated_filenames(
    left: &mut [Part],
    right: &mut [Part],
    selected: &[usize],
) -> Result<(), usize> {
    selected.iter().try_for_each(|part| {
        let pair = left.get_mut(*part).zip(right.get_mut(*part));
        let Some((left_part, right_part)) = pair else {
            return Ok(());
        };
        let names = (
            left_part.filename.as_deref(),
            right_part.filename.as_deref(),
        );
        let (Some(left_name), Some(right_name)) = names else {
            return Err(*part);
        };
        if !generated_filenames_match(left_name, right_name) {
            return Err(*part);
        }
        left_part.filename = Some(String::new());
        right_part.filename = Some(String::new());
        Ok(())
    })
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
