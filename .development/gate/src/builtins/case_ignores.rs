use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::builtins::Context;
use crate::error::GateError;

const CASES: &str = "tools/oracle/cases/";
// the fields of HelperOutput in tools/oracle/src/compare/helper.rs
const ANSWER_FIELDS: [&str; 5] = ["ok", "value", "error", "warnings", "effects"];

#[derive(Debug, Deserialize)]
struct CaseHead {
    kind: Option<String>,
    #[serde(default)]
    compare: CompareHead,
}

#[derive(Debug, Default, Deserialize)]
struct CompareHead {
    #[serde(default)]
    ignore: Vec<IgnoreHead>,
}

#[derive(Debug, Deserialize)]
struct IgnoreHead {
    pointer: String,
}

pub(crate) fn pointer_problems(path: &str, text: &str) -> Result<Vec<String>, serde_saphyr::Error> {
    let case: CaseHead = serde_saphyr::from_str(text)?;
    if case.kind.as_deref() != Some("helper") {
        return Ok(Vec::new());
    }
    Ok(case
        .compare
        .ignore
        .iter()
        .filter(|ignored| {
            let field = ignored
                .pointer
                .strip_prefix('/')
                .and_then(|rest| rest.split('/').next());
            !ignored.pointer.is_empty()
                && !field.is_some_and(|name| ANSWER_FIELDS.contains(&name))
        })
        .map(|ignored| {
            format!(
                "{path}: compare.ignore pointer {:?} names no field of a helper answer; start it with /{}",
                ignored.pointer,
                ANSWER_FIELDS.join(", /")
            )
        })
        .collect())
}

pub(crate) fn check(context: &Context<'_>) -> Result<Vec<String>, GateError> {
    let root = &context.development.root;
    let mut problems = Vec::new();
    for path in &context.facts.changed {
        let file = root.join(path);
        let yaml = Path::new(path)
            .extension()
            .is_some_and(|extension| extension == "yaml");
        if !path.starts_with(CASES) || !yaml || !file.is_file() {
            continue;
        }
        let text = fs::read_to_string(&file).map_err(|source| GateError::Read {
            path: file.clone(),
            source,
        })?;
        let found = pointer_problems(path, &text).map_err(|source| GateError::Yaml {
            path: file,
            source: Box::new(source),
        })?;
        problems.extend(found);
    }
    Ok(problems)
}
