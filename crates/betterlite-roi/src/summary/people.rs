use indexmap::IndexMap;
use rust_decimal::Decimal;

use crate::records::{ROIEstimateStatus, ROISpendRecord};
use crate::settings::normalize_email;
use crate::summary::error::SummaryError;
use crate::summary::identity::{pull_key, spend_key};
use crate::summary::models::{ROIPersonSummary, ROIPullSummary};

#[derive(Default)]
pub(super) struct PersonRows<'report> {
    spend: Vec<&'report ROISpendRecord>,
    pulls: Vec<&'report ROIPullSummary>,
}

pub(super) fn group_people<'report>(
    spend: &'report [ROISpendRecord],
    pulls: &'report [ROIPullSummary],
) -> IndexMap<String, PersonRows<'report>> {
    let mut people: IndexMap<String, PersonRows<'report>> = IndexMap::new();
    for row in spend {
        people.entry(spend_key(row)).or_default().spend.push(row);
    }
    for pull in pulls {
        let key = pull_key(&pull.login, &pull.email);
        people.entry(key).or_default().pulls.push(pull);
    }
    people
}

pub(super) fn summarize_person(
    id: String,
    rows: &PersonRows<'_>,
    complete_scope: bool,
) -> Result<ROIPersonSummary, SummaryError> {
    // LiteLLM takes the first spend row with any email, so an invalid one leaves the person without an email
    let email = rows
        .spend
        .iter()
        .find(|row| !row.email.is_empty())
        .map(|row| normalize_email(&row.email))
        .or_else(|| rows.pulls.first().map(|pull| pull.email.clone()))
        .unwrap_or_default();
    let spend = spend_total(&rows.spend)?;
    let (hours, estimated_prs) = estimate_totals(&rows.pulls)?;
    let pending_prs = rows
        .pulls
        .iter()
        .filter(|pull| pull.estimate.status != ROIEstimateStatus::Estimated)
        .count();
    let eligible = spend.is_some() && estimated_prs > 0 && pending_prs == 0;
    let cost_per_hour = spend
        .filter(|_| complete_scope && eligible && hours > Decimal::ZERO)
        .map(|total| {
            total.checked_div(hours).ok_or(SummaryError::Overflow {
                figure: "cost per hour",
            })
        })
        .transpose()?;
    Ok(ROIPersonSummary {
        id,
        email,
        logins: rows.pulls.iter().map(|pull| pull.login.clone()).collect(),
        spend,
        hours,
        prs: rows.pulls.len(),
        estimated_prs,
        pending_prs,
        match_methods: rows.pulls.iter().map(|pull| pull.match_method).collect(),
        eligible,
        cost_per_hour,
    })
}

fn spend_total(rows: &[&ROISpendRecord]) -> Result<Option<Decimal>, SummaryError> {
    if rows.is_empty() {
        return Ok(None);
    }
    let total = rows
        .iter()
        .map(|row| row.spend)
        .try_fold(Decimal::ZERO, Decimal::checked_add)
        .ok_or(SummaryError::Overflow { figure: "spend" })?;
    Ok(Some(total))
}

fn estimate_totals(pulls: &[&ROIPullSummary]) -> Result<(Decimal, usize), SummaryError> {
    let mut hours = Decimal::ZERO;
    let mut estimated = 0_usize;
    for pull in pulls {
        if pull.estimate.status != ROIEstimateStatus::Estimated {
            continue;
        }
        // LiteLLM counts an estimate without hours as zero hours
        if let Some(value) = pull.estimate.hours {
            hours = hours
                .checked_add(value)
                .ok_or(SummaryError::Overflow { figure: "hours" })?;
        }
        estimated = estimated.checked_add(1).ok_or(SummaryError::Overflow {
            figure: "estimated pulls",
        })?;
    }
    Ok((hours, estimated))
}
