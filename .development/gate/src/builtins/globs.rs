use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

use crate::error::GateError;

pub(crate) fn glob_set(patterns: &[String]) -> Result<GlobSet, GateError> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let expanded = if pattern.ends_with('/') {
            format!("{pattern}**")
        } else {
            pattern.clone()
        };
        let glob = GlobBuilder::new(&expanded)
            .literal_separator(true)
            .build()
            .map_err(|source| GateError::Glob {
                pattern: pattern.clone(),
                source: Box::new(source),
            })?;
        builder.add(glob);
    }
    builder.build().map_err(|source| GateError::Glob {
        pattern: patterns.join(", "),
        source: Box::new(source),
    })
}
