use super::{CatalogError, MAX_CATALOG_WARNING_BYTES, MAX_CATALOG_WARNINGS};

pub(super) struct WarningState {
    pub(super) messages: Vec<String>,
    pub(super) bytes: usize,
}

impl WarningState {
    pub(super) fn push<Message>(&mut self, message: Message) -> Result<(), CatalogError>
    where
        Message: FnOnce() -> String,
    {
        let warning = message();
        self.reserve(warning.len())?;
        self.messages.push(warning);
        Ok(())
    }

    pub(super) fn collision(
        &mut self,
        key: &str,
        kept: &str,
        canonical: &str,
    ) -> Result<(), CatalogError> {
        let size = "alias collision for "
            .len()
            .saturating_add(key.len())
            .saturating_add(": kept ".len())
            .saturating_add(kept.len())
            .saturating_add(", ignored ".len())
            .saturating_add(canonical.len());
        self.reserve(size)?;
        self.messages.push(format!(
            "alias collision for {key}: kept {kept}, ignored {canonical}"
        ));
        Ok(())
    }

    fn reserve(&mut self, size: usize) -> Result<(), CatalogError> {
        if self.messages.len() >= MAX_CATALOG_WARNINGS {
            return Err(CatalogError::TooManyWarnings {
                limit: MAX_CATALOG_WARNINGS,
            });
        }
        let total = self
            .bytes
            .checked_add(size)
            .ok_or(CatalogError::TooManyWarningBytes {
                limit: MAX_CATALOG_WARNING_BYTES,
            })?;
        if total > MAX_CATALOG_WARNING_BYTES {
            return Err(CatalogError::TooManyWarningBytes {
                limit: MAX_CATALOG_WARNING_BYTES,
            });
        }
        self.bytes = total;
        Ok(())
    }
}
