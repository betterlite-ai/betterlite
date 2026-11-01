use base64::Engine as _;

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) stored_content: String,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let content = base64::engine::general_purpose::STANDARD
            .decode(&input.stored_content)
            .map_err(|error| {
                Box::new(crate::helpers::HelperError {
                    class: "ValueError".to_owned(),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: format!("stored_content must be base64: {error}"),
                })
            })?;
        let Some(archive) = betterlite_http::skill_archive::build_skill_archive(&content) else {
            return Ok(serde_json::Value::Null);
        };
        Ok(serde_json::json!({
            "content": base64::engine::general_purpose::STANDARD.encode(archive.content),
            "digest": archive.digest,
            "declared_name": archive.declared_name,
            "declared_description": archive.declared_description,
        }))
    }
}
