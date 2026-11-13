use std::path::Path;

use secrecy::{ExposeSecret as _, SecretString};
use tokio::fs;

use crate::hub_oauth::error::{RefreshError, StoredTokensError};
use crate::hub_oauth::secret_file::write_secret;
use crate::hub_oauth::stored_tokens::parse::{ParseError, Section};

pub mod parse;

// huggingface_hub's _read_stored_tokens_full: a missing or unreadable file and a parse error give no sections, bytes that are not UTF-8 are an error
pub async fn read_full(path: &Path) -> Result<Vec<Section>, StoredTokensError> {
    let Ok(bytes) = fs::read(path).await else {
        return Ok(Vec::new());
    };
    match parse::parse(&bytes) {
        Ok(file) => Ok(file.with_defaults()),
        Err(ParseError::Utf8(source)) => Err(StoredTokensError::Utf8 {
            path: path.to_path_buf(),
            source,
        }),
        Err(error) => {
            tracing::error!(%error, "Error parsing stored tokens file");
            Ok(Vec::new())
        }
    }
}

#[must_use]
pub fn field<'section>(section: &'section Section, name: &str) -> Option<&'section SecretString> {
    section
        .fields
        .iter()
        .find_map(|(known, value)| (known == name).then_some(value))
}

// ConfigParser.write with interpolation off: sections by name, "key = value", continuation lines tab-indented
fn render(sections: &[Section]) -> String {
    let mut ordered: Vec<&Section> = sections.iter().collect();
    ordered.sort_by(|left, right| left.name.cmp(&right.name));
    let mut text = String::new();
    for section in ordered {
        text.push('[');
        text.push_str(&section.name);
        text.push_str("]\n");
        for (name, value) in &section.fields {
            text.push_str(name);
            text.push_str(" = ");
            text.push_str(&value.expose_secret().replace('\n', "\n\t"));
            text.push('\n');
        }
        text.push('\n');
    }
    text
}

// huggingface_hub's _save_token: the named section is replaced whole so a new login drops stale metadata
pub async fn save_token(
    path: &Path,
    name: &str,
    token: &SecretString,
    refresh_token: &SecretString,
    expires_at: Option<i64>,
) -> Result<(), RefreshError> {
    let mut sections = read_full(path).await?;
    let mut fields = vec![
        ("hf_token".to_owned(), token.clone()),
        ("refresh_token".to_owned(), refresh_token.clone()),
    ];
    if let Some(expiry) = expires_at {
        fields.push((
            "expires_at".to_owned(),
            SecretString::from(expiry.to_string()),
        ));
    }
    sections.retain(|section| section.name != name);
    sections.push(Section {
        name: name.to_owned(),
        fields,
    });
    write_secret(path, &render(&sections))
        .await
        .map_err(RefreshError::Save)
}
