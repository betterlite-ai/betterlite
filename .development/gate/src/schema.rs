use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use schemars::{Schema, schema_for};

use crate::error::GateError;
use crate::spec::agents::Agents;
use crate::spec::backlog::Backlog;
use crate::spec::catalog::Catalog;
use crate::spec::litellm::Litellm;
use crate::spec::report::Report;
use crate::spec::review::Review;
use crate::spec::rules::Rules;
use crate::spec::task::Task;

pub(crate) fn generate() -> Result<Vec<(&'static str, String)>, GateError> {
    let schemas: [(&'static str, Schema); 8] = [
        ("task.schema.json", schema_for!(Task)),
        ("backlog.schema.json", schema_for!(Backlog)),
        ("checks.schema.json", schema_for!(Catalog)),
        ("rules.schema.json", schema_for!(Rules)),
        ("agents.schema.json", schema_for!(Agents)),
        ("report.schema.json", schema_for!(Report)),
        ("review.schema.json", schema_for!(Review)),
        ("litellm.schema.json", schema_for!(Litellm)),
    ];
    schemas
        .into_iter()
        .map(|(name, schema)| {
            serde_json::to_string_pretty(&schema)
                .map(|text| (name, format!("{text}\n")))
                .map_err(|source| GateError::Json {
                    what: "schemars".to_owned(),
                    source,
                })
        })
        .collect()
}

pub(crate) fn stale(root: &Path) -> Result<Vec<String>, GateError> {
    let mut problems = Vec::new();
    for (name, text) in generate()? {
        let path = root.join(".development/schema").join(name);
        let committed = match fs::read_to_string(&path) {
            Ok(committed) => committed,
            Err(source) if source.kind() == ErrorKind::NotFound => String::new(),
            Err(source) => return Err(GateError::Read { path, source }),
        };
        if committed != text {
            problems.push(format!(
                ".development/schema/{name} is out of date; run cargo gate schema"
            ));
        }
    }
    Ok(problems)
}

pub(crate) fn write(root: &Path) -> Result<Vec<String>, GateError> {
    let mut lines = Vec::new();
    for (name, text) in generate()? {
        let path = root.join(".development/schema").join(name);
        fs::write(&path, text).map_err(|source| GateError::Write { path, source })?;
        lines.push(format!("wrote .development/schema/{name}"));
    }
    Ok(lines)
}
