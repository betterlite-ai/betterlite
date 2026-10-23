use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

use crate::error::GateError;

pub(crate) fn glob_set(patterns: &[String]) -> Result<GlobSet, GateError> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let glob = GlobBuilder::new(pattern)
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
