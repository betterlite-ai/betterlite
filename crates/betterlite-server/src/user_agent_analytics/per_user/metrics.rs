use std::cmp::Reverse;
use std::collections::HashMap;
use std::collections::HashSet;

use rust_decimal::Decimal;

use super::super::error::AnalyticsError;
use super::super::query::Pagination;
use super::super::responses::PerUserAnalyticsResponse;
use super::super::responses::PerUserMetric;
use super::super::rows::TagSpendRecord;
use super::super::rows::TokenOwnerRow;
use super::super::rows::UserEmailRow;

impl PerUserMetric {
    fn new(user_id: &str, user_email: Option<&str>, tag: Option<&str>) -> Self {
        Self {
            user_id: user_id.to_owned(),
            user_email: user_email.map(str::to_owned),
            user_agent: tag.map(str::to_owned),
            successful_requests: 0,
            failed_requests: 0,
            total_requests: 0,
            total_tokens: 0,
            spend: Decimal::ZERO,
        }
    }

    fn observe(&mut self, record: &TagSpendRecord) -> Result<(), AnalyticsError> {
        if self.user_agent.is_none() {
            self.user_agent.clone_from(&record.tag);
        }
        let tokens = record
            .prompt_tokens
            .saturating_add(record.completion_tokens);
        self.successful_requests = self
            .successful_requests
            .saturating_add(record.successful_requests);
        self.failed_requests = self.failed_requests.saturating_add(record.failed_requests);
        self.total_requests = self.total_requests.saturating_add(record.api_requests);
        self.total_tokens = self.total_tokens.saturating_add(tokens);
        self.spend = self
            .spend
            .checked_add(record.spend)
            .ok_or(AnalyticsError::Overflow)?;
        Ok(())
    }
}

pub(super) fn api_keys(records: &[TagSpendRecord]) -> Vec<String> {
    let mut keys = Vec::new();
    let mut seen = HashSet::new();
    for record in records {
        if record.api_key.is_empty() {
            continue;
        }
        if seen.insert(record.api_key.as_str()) {
            keys.push(record.api_key.clone());
        }
    }
    keys
}

pub(super) fn user_ids(owners: &[TokenOwnerRow]) -> Vec<String> {
    let mut ids = Vec::new();
    let mut seen = HashSet::new();
    for row in owners {
        if seen.insert(row.user_id.as_str()) {
            ids.push(row.user_id.clone());
        }
    }
    ids
}

pub(super) fn aggregate(
    records: &[TagSpendRecord],
    owners: &[TokenOwnerRow],
    emails: &[UserEmailRow],
) -> Result<Vec<PerUserMetric>, AnalyticsError> {
    let mut results = totals(records, owners, emails)?;
    results.sort_by_key(|row| Reverse(row.successful_requests));
    Ok(results)
}

fn totals(
    records: &[TagSpendRecord],
    owners: &[TokenOwnerRow],
    emails: &[UserEmailRow],
) -> Result<Vec<PerUserMetric>, AnalyticsError> {
    let owner_of: HashMap<&str, &str> = owners
        .iter()
        .map(|row| (row.token.as_str(), row.user_id.as_str()))
        .collect();
    let email_of: HashMap<&str, Option<&str>> = emails
        .iter()
        .map(|row| (row.user_id.as_str(), row.user_email.as_deref()))
        .collect();
    let mut per_user: Vec<PerUserMetric> = Vec::new();
    let mut positions: HashMap<&str, usize> = HashMap::new();
    for record in records {
        let Some(owner) = owner_of.get(record.api_key.as_str()).copied() else {
            continue;
        };
        let index = match positions.get(owner).copied() {
            Some(index) => index,
            None => {
                let index = per_user.len();
                positions.insert(owner, index);
                per_user.push(PerUserMetric::new(
                    owner,
                    email_of.get(owner).copied().flatten(),
                    record.tag.as_deref(),
                ));
                index
            }
        };
        let Some(total) = per_user.get_mut(index) else {
            return Err(AnalyticsError::Overflow);
        };
        total.observe(record)?;
    }
    Ok(per_user)
}

pub(super) fn page(
    results: Vec<PerUserMetric>,
    pagination: Pagination,
) -> PerUserAnalyticsResponse {
    let total_count = i64::try_from(results.len()).unwrap_or(i64::MAX);
    let size = usize::try_from(pagination.page_size)
        .unwrap_or(usize::MAX)
        .max(1);
    let total_pages = results.len().div_ceil(size);
    let start = pagination
        .page
        .saturating_sub(1)
        .saturating_mul(pagination.page_size);
    let offset = usize::try_from(start).unwrap_or(usize::MAX);
    PerUserAnalyticsResponse {
        results: results.into_iter().skip(offset).take(size).collect(),
        total_count,
        page: pagination.page,
        page_size: pagination.page_size,
        total_pages: i64::try_from(total_pages).unwrap_or(i64::MAX),
    }
}
