use super::error::VertexSchemaError;

#[derive(Debug, Clone, Copy)]
pub(crate) struct Recursion {
    pub(crate) max_depth: usize,
}

impl Recursion {
    pub(crate) const fn check(self, depth: usize) -> Result<(), VertexSchemaError> {
        if depth > self.max_depth {
            return Err(VertexSchemaError::DepthLimitExceeded {
                limit: self.max_depth,
            });
        }
        Ok(())
    }
}
