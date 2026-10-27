use std::collections::BTreeSet;

use indexmap::IndexMap;
use serde::Serialize;

use crate::records::{ROIPullRecord, ROISpendRecord};
use crate::settings::normalize_email;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum MatchMethod {
    #[serde(rename = "manual")]
    Manual,
    #[serde(rename = "profile email")]
    ProfileEmail,
    #[serde(rename = "commit email")]
    CommitEmail,
    #[serde(rename = "ambiguous emails")]
    AmbiguousEmails,
    #[serde(rename = "email unavailable")]
    EmailUnavailable,
    #[serde(rename = "no gateway match")]
    NoGatewayMatch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullIdentity {
    pub email: String,
    pub method: MatchMethod,
}

#[must_use]
pub fn match_identity(
    pull: &ROIPullRecord,
    observed_emails: &BTreeSet<String>,
    mappings: &IndexMap<String, String>,
) -> PullIdentity {
    let login = caseless::default_case_fold_str(&pull.login);
    if let Some(mapped) = mappings.get(&login).filter(|mapped| !mapped.is_empty()) {
        return PullIdentity {
            email: normalize_email(mapped),
            method: MatchMethod::Manual,
        };
    }
    let candidates: BTreeSet<String> = pull
        .emails
        .iter()
        .map(|candidate| normalize_email(candidate))
        .filter(|address| !address.is_empty())
        .collect();
    let mut matched = candidates.intersection(observed_emails);
    let method = match (matched.next(), matched.next()) {
        (Some(address), None) => {
            let method = if *address == normalize_email(&pull.profile_email) {
                MatchMethod::ProfileEmail
            } else {
                MatchMethod::CommitEmail
            };
            return PullIdentity {
                email: address.clone(),
                method,
            };
        }
        (Some(_), Some(_)) => MatchMethod::AmbiguousEmails,
        (None, _) if candidates.is_empty() => MatchMethod::EmailUnavailable,
        (None, _) => MatchMethod::NoGatewayMatch,
    };
    PullIdentity {
        email: String::new(),
        method,
    }
}

pub(super) fn spend_key(row: &ROISpendRecord) -> String {
    let email = normalize_email(&row.email);
    if email.is_empty() {
        format!("gateway:{}", row.user_id)
    } else {
        email
    }
}

pub(super) fn pull_key(login: &str, email: &str) -> String {
    if email.is_empty() {
        format!("github:{}", caseless::default_case_fold_str(login))
    } else {
        email.to_owned()
    }
}
