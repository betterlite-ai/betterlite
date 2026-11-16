use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_http::skill_archive;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) stored_content: String,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let content = STANDARD.decode(&input.stored_content).map_err(|error| {
            Box::new(HelperError {
                class: "ValueError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: format!("stored_content must be base64: {error}"),
            })
        })?;
        let Some(archive) = skill_archive::build_skill_archive(&content) else {
            return Ok(serde_json::json!({ "archive": null }));
        };
        Ok(serde_json::json!({
            "archive": {
                "content": STANDARD.encode(archive.content),
                "digest": archive.digest,
                "declared_name": archive.declared_name,
                "declared_description": archive.declared_description,
            }
        }))
    }
}
