use rust_decimal::Decimal;

use crate::records::ROIEstimateStatus;
use crate::summary::error::SummaryError;
use crate::summary::models::{ROIPersonSummary, ROIPullSummary, ROISummaryMetrics};

pub(super) fn summary_metrics(
    people: &[ROIPersonSummary],
    pulls: &[ROIPullSummary],
    complete_scope: bool,
) -> Result<ROISummaryMetrics, SummaryError> {
    let cohort: Vec<&ROIPersonSummary> = people.iter().filter(|person| person.eligible).collect();
    let (matched_spend, output_hours) = spend_and_hours(cohort.iter().copied())?;
    // every spend row belongs to exactly one person, so this equals the sum over the report's rows
    let (total_spend, total_output_hours) = spend_and_hours(people)?;
    let excluded_spend = total_spend
        .checked_sub(matched_spend)
        .ok_or(SummaryError::Overflow {
            figure: "excluded spend",
        })?;
    let is_estimated =
        |pull: &&ROIPullSummary| pull.estimate.status == ROIEstimateStatus::Estimated;
    Ok(ROISummaryMetrics {
        matched_spend,
        output_hours,
        total_spend,
        total_output_hours,
        excluded_spend: excluded_spend.max(Decimal::ZERO),
        cost_per_hour: ratio(matched_spend, output_hours, complete_scope)?,
        hours_per_dollar: ratio(output_hours, matched_spend, complete_scope)?,
        merged_prs: pulls.len(),
        estimated_prs: pulls.iter().filter(is_estimated).count(),
        matched_prs: pulls.iter().filter(|pull| pull.matched).count(),
        cohort_people: cohort.len(),
        people_with_prs: people.iter().filter(|person| person.prs > 0).count(),
        pending_prs: pulls.iter().filter(|pull| !is_estimated(pull)).count(),
    })
}

fn spend_and_hours<'person, I>(people: I) -> Result<(Decimal, Decimal), SummaryError>
where
    I: IntoIterator<Item = &'person ROIPersonSummary>,
{
    let mut spend = Decimal::ZERO;
    let mut hours = Decimal::ZERO;
    for person in people {
        if let Some(amount) = person.spend {
            spend = spend.checked_add(amount).ok_or(SummaryError::Overflow {
                figure: "spend total",
            })?;
        }
        hours = hours
            .checked_add(person.hours)
            .ok_or(SummaryError::Overflow {
                figure: "hours total",
            })?;
    }
    Ok((spend, hours))
}

fn ratio(
    numerator: Decimal,
    denominator: Decimal,
    complete_scope: bool,
) -> Result<Option<Decimal>, SummaryError> {
    if !complete_scope || denominator.is_zero() {
        return Ok(None);
    }
    let value = numerator
        .checked_div(denominator)
        .ok_or(SummaryError::Overflow { figure: "ratio" })?;
    Ok(Some(value.normalize()))
}
