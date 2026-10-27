use std::collections::{BTreeMap, BTreeSet};

use rust_decimal::Decimal;

use crate::records::{ROIEstimateStatus, ROISpendRecord};
use crate::settings::normalize_email;
use crate::summary::error::SummaryError;
use crate::summary::models::{ROIPersonSummary, ROIPullSummary, ROITrendDay};

pub(super) fn daily_trend(
    spend: &[ROISpendRecord],
    pulls: &[ROIPullSummary],
    people: &[ROIPersonSummary],
) -> Result<Vec<ROITrendDay>, SummaryError> {
    let eligible: BTreeSet<&str> = people
        .iter()
        .filter(|person| person.eligible)
        .map(|person| person.email.as_str())
        .collect();
    // a pull's day is the first ten characters of merged_at, the date part of its timestamp
    let pull_days: Vec<String> = pulls
        .iter()
        .map(|pull| pull.merged_at.chars().take(10).collect())
        .collect();
    let mut days = empty_days(spend, &pull_days);
    for row in spend {
        if let Some(day) = days.get_mut(row.date.as_str())
            && eligible.contains(normalize_email(&row.email).as_str())
        {
            day.spend = day
                .spend
                .checked_add(row.spend)
                .ok_or(SummaryError::Overflow {
                    figure: "daily spend",
                })?;
        }
    }
    for (pull, date) in pulls.iter().zip(&pull_days) {
        if let Some(day) = days.get_mut(date.as_str())
            && eligible.contains(pull.email.as_str())
            && pull.estimate.status == ROIEstimateStatus::Estimated
        {
            add_pull(day, pull)?;
        }
    }
    Ok(days.into_values().collect())
}

fn empty_days<'date>(
    spend: &'date [ROISpendRecord],
    pull_days: &'date [String],
) -> BTreeMap<&'date str, ROITrendDay> {
    let dates = spend.iter().map(|row| &row.date).chain(pull_days);
    dates
        .map(|date| {
            let day = ROITrendDay {
                date: date.clone(),
                spend: Decimal::ZERO,
                hours: Decimal::ZERO,
                prs: 0,
            };
            (date.as_str(), day)
        })
        .collect()
}

fn add_pull(day: &mut ROITrendDay, pull: &ROIPullSummary) -> Result<(), SummaryError> {
    if let Some(hours) = pull.estimate.hours {
        day.hours = day.hours.checked_add(hours).ok_or(SummaryError::Overflow {
            figure: "daily hours",
        })?;
    }
    day.prs = day.prs.checked_add(1).ok_or(SummaryError::Overflow {
        figure: "daily pulls",
    })?;
    Ok(())
}
