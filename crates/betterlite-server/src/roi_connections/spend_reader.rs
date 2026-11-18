use crate::roi_settings::repository::DatabaseSettingsRepository;
use crate::roi_spend::PostgresSpendReader;

#[must_use]
pub fn spend_reader(repository: &DatabaseSettingsRepository) -> PostgresSpendReader {
    PostgresSpendReader {
        store: repository.store.as_ref().clone(),
    }
}

mod tests;
