use crate::error_body::ModelError;
use std::collections::BTreeMap;
pub(crate) fn read_query<const COUNT: usize>(
    raw_query: Option<&str>,
    flag_names: [&'static str; COUNT],
) -> Result<(BTreeMap<String, String>, [bool; COUNT]), ModelError> {
    // Starlette keeps the last value of a repeated parameter, and so does collecting into a map
    let values: BTreeMap<String, String> =
        form_urlencoded::parse(raw_query.unwrap_or_default().as_bytes())
            .into_owned()
            .collect();
    let mut flags = [false; COUNT];
    let mut invalid = Vec::new();
    for (flag, name) in flags.iter_mut().zip(flag_names) {
        // the strings pydantic accepts as a boolean, without trimming
        match values
            .get(name)
            .map(|value| value.to_ascii_lowercase())
            .as_deref()
        {
            None | Some("0" | "off" | "f" | "false" | "n" | "no") => {}
            Some("1" | "on" | "t" | "true" | "y" | "yes") => *flag = true,
            Some(_) => invalid.push(name),
        }
    }
    if invalid.is_empty() {
        return Ok((values, flags));
    }
    Err(ModelError::InvalidFlags(invalid))
}
