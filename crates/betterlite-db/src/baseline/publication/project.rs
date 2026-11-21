use betterlite_models::baseline_history::error::BaselineHistoryError;
use betterlite_models::baseline_history::model::{BaselineHistory, BaselineObservation, Outcome};
use betterlite_models::baseline_history::replay::advance_baseline_history;
use betterlite_models::savings_pricing::context::SavingsPricingContext;

use super::change::{Change, change};
use super::decode::{PublicationDecodeError, decode_baseline_publication};
use super::pricing::{PublicationPricing, baseline_publication};
use crate::baseline::records::{
    BaselineAccountingRecord, RecordDecodeError, decode_baseline_record,
};

// One row of the observation page the group is projected from.
#[derive(Clone, Debug)]
pub struct StoredRecord {
    pub data: String,
    pub publication: Option<String>,
    pub conflicted: bool,
    pub started_at: f64,
}

#[derive(Debug, thiserror::Error)]
pub enum ProjectionError {
    #[error("stored baseline record cannot be decoded")]
    Record(#[from] RecordDecodeError),
    #[error("stored baseline publication cannot be decoded")]
    Publication(#[from] PublicationDecodeError),
    #[error("baseline history cannot advance")]
    History(#[from] BaselineHistoryError),
}

// _project_group: replay the page over the previous history and append only the publications that moved.
pub fn project_group<C>(
    pricing: &PublicationPricing<'_, C>,
    previous: &(BaselineHistory, Vec<Change>),
    stored: &[StoredRecord],
) -> Result<(BaselineHistory, Vec<Change>), ProjectionError>
where
    C: SavingsPricingContext,
{
    let records = stored
        .iter()
        .map(|row| decode_baseline_record(row.data.as_bytes()))
        .collect::<Result<Vec<_>, _>>()?;
    let observations = records
        .iter()
        .zip(stored)
        .map(observation)
        .collect::<Vec<_>>();
    let (advanced, estimates) = advance_baseline_history(&previous.0, &observations)?;
    let first_at = advanced.first_at.unwrap_or_else(|| {
        observations
            .first()
            .map_or(0.0_f64, |value| value.started_at)
    });
    let mut changes = previous.1.clone();
    for ((record, row), estimate) in records.iter().zip(stored).zip(&estimates) {
        let publication = baseline_publication(pricing, record, estimate, first_at);
        let old = row
            .publication
            .as_deref()
            .map(|text| decode_baseline_publication(text.as_bytes()))
            .transpose()?;
        if old.as_ref() != Some(&publication) {
            changes.push(change(record, old.as_ref(), &publication));
        }
    }
    Ok((advanced, changes))
}

// A conflicted observation becomes the uncertain record that replaces what the group would have reported.
fn observation((record, row): (&BaselineAccountingRecord, &StoredRecord)) -> BaselineObservation {
    let mut observation = record.observation.clone();
    if row.conflicted {
        observation.outcome = Outcome::Uncertain;
        observation.baseline_equivalent = false;
        observation.reason = Some("conflicting_observation".to_owned());
    }
    observation
}
