use betterlite_secrets::secret_codec::CALLBACK_VAR_ENCRYPTED_PREFIX;
use secrecy::{ExposeSecret as _, SecretString};

use super::MigrationError;
use crate::encryption_scan::secret_text::SecretText;
use crate::encryption_scan::MigrationJson;

enum Stage {
    Visit(MigrationJson, i64),
    CollectArray(usize),
    CollectObject(Vec<String>),
}

struct Builder {
    stages: Vec<Stage>,
    results: Vec<MigrationJson>,
    count: usize,
    max_depth: i64,
}

type Replacement<'a> = &'a mut dyn FnMut(&str) -> Result<Option<String>, MigrationError>;

impl Builder {
    fn visit(
        &mut self,
        value: MigrationJson,
        depth: i64,
        replacement_for: Replacement<'_>,
    ) -> Result<(), MigrationError> {
        if depth > self.max_depth {
            self.results.push(value);
            return Ok(());
        }
        match value {
            MigrationJson::String(text) => self.visit_text(&text, replacement_for),
            MigrationJson::Array(items) => {
                let next = depth.saturating_add(1);
                self.stages.push(Stage::CollectArray(self.results.len()));
                self.stages
                    .extend(items.into_iter().rev().map(|item| Stage::Visit(item, next)));
                Ok(())
            }
            MigrationJson::Object(entries) => {
                let next = depth.saturating_add(1);
                let keys = entries.keys().cloned().collect();
                self.stages.push(Stage::CollectObject(keys));
                self.stages.extend(
                    entries
                        .into_iter()
                        .rev()
                        .map(|(_, item)| Stage::Visit(item, next)),
                );
                Ok(())
            }
            MigrationJson::Null | MigrationJson::Bool(_) | MigrationJson::Number(_) => {
                self.results.push(value);
                Ok(())
            }
        }
    }

    fn visit_text(
        &mut self,
        text: &SecretText,
        replacement_for: Replacement<'_>,
    ) -> Result<(), MigrationError> {
        let exposed = text.0.expose_secret().to_owned();
        let (marker, inner) = exposed
            .strip_prefix(CALLBACK_VAR_ENCRYPTED_PREFIX)
            .map_or(("", exposed.as_str()), |rest| {
                (CALLBACK_VAR_ENCRYPTED_PREFIX, rest)
            });
        let replaced = match replacement_for(inner)? {
            Some(replacement) => {
                self.count = self.count.saturating_add(1);
                format!("{marker}{replacement}")
            }
            None => exposed,
        };
        self.results
            .push(MigrationJson::String(SecretText(SecretString::from(
                replaced,
            ))));
        Ok(())
    }

    fn finish(mut self) -> Result<(MigrationJson, usize), MigrationError> {
        let (Some(rebuilt), true) = (self.results.pop(), self.results.is_empty()) else {
            return Err(MigrationError::Unrebuildable);
        };
        Ok((rebuilt, self.count))
    }
}

// the pending stages hold the depth, so a configured depth past the input's nesting never recurses
pub fn replace_ciphertexts<R>(
    value: MigrationJson,
    replacement_for: &mut R,
    depth: i64,
    max_depth: i64,
) -> Result<(MigrationJson, usize), MigrationError>
where
    R: FnMut(&str) -> Result<Option<String>, MigrationError>,
{
    let mut builder = Builder {
        stages: vec![Stage::Visit(value, depth)],
        results: Vec::new(),
        count: 0,
        max_depth,
    };
    while let Some(stage) = builder.stages.pop() {
        match stage {
            Stage::Visit(stored, at) => builder.visit(stored, at, &mut *replacement_for)?,
            Stage::CollectArray(start) => {
                let children = builder.results.split_off(start);
                builder.results.push(MigrationJson::Array(children));
            }
            Stage::CollectObject(keys) => {
                let start = builder.results.len().saturating_sub(keys.len());
                let children = builder.results.split_off(start);
                builder.results.push(MigrationJson::Object(
                    keys.into_iter().zip(children).collect(),
                ));
            }
        }
    }
    builder.finish()
}

// LiteLLM sends json.dumps(value) for a JSON column and a str-validated value for a scalar one
pub fn as_sql_parameter(value: &MigrationJson, is_json: bool) -> Result<String, MigrationError> {
    if is_json {
        return serde_json::to_string(value).map_err(MigrationError::Json);
    }
    match value {
        MigrationJson::String(SecretText(text)) => Ok(text.expose_secret().to_owned()),
        MigrationJson::Null
        | MigrationJson::Bool(_)
        | MigrationJson::Number(_)
        | MigrationJson::Array(_)
        | MigrationJson::Object(_) => Err(MigrationError::NotText),
    }
}
