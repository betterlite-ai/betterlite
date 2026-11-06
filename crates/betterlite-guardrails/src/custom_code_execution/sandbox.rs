pub mod error;
pub mod globals;
pub mod rewrite;

use error::CompileError;

pub const DEFAULT_FILENAME: &str = "<guardrail>";

// The compiled guardrail program: guarded source plus its module-level functions, loaded once per guardrail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledProgram {
    pub source: String,
    pub filename: String,
    pub functions: Vec<rewrite::Function>,
}

impl CompiledProgram {
    #[must_use]
    pub fn is_async(&self, name: &str) -> Option<bool> {
        self.functions
            .iter()
            .find(|function| function.name == name)
            .map(|function| function.is_async)
    }
}

// Compiles once: syntax errors and forbidden constructs surface here, and the loop guards are inserted.
pub fn compile_sandboxed(source: &str, filename: &str) -> Result<CompiledProgram, CompileError> {
    let tree = rewrite::parse(source, filename)?;
    let analysis = rewrite::analyze(&tree, source, filename)?;
    let rendered = rewrite::apply(source, &analysis.edits, filename)?;
    Ok(CompiledProgram {
        source: rendered,
        filename: filename.to_owned(),
        functions: analysis.functions,
    })
}
