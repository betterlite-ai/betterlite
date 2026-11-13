use crate::cohere::error::CohereError;

// The same 8 MiB ceiling B232's message parser applies, shared by every recovery of one conversion
pub const MAX_ARGUMENT_BYTES: usize = 0x0080_0000;

pub(crate) struct ArgumentBudget {
    pub(crate) spent: usize,
}

impl ArgumentBudget {
    pub(crate) const fn charge(&mut self, bytes: usize) -> Result<(), CohereError> {
        let total = self.spent.saturating_add(bytes);
        if total > MAX_ARGUMENT_BYTES {
            return Err(CohereError::ArgumentBudget {
                limit: MAX_ARGUMENT_BYTES,
            });
        }
        self.spent = total;
        Ok(())
    }
}
