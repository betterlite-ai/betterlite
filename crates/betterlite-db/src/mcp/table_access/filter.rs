use super::types::{ACTIVE, IdentifierField};

// NULL predates the approval workflow and draft rows back a transient OAuth session
pub const NOT_DRAFT_PREDICATE: &str =
    "(\"approval_status\" IS NULL OR \"approval_status\" <> 'draft')";

#[derive(Debug, Clone)]
pub enum ServerFilter {
    ServerId(String),
    ServerIds(Vec<String>),
    Identifiers {
        value: String,
        exclude_server_id: Option<String>,
    },
    ApprovalStatus(String),
    NotDraft,
    SubmittedByActive {
        user_id: String,
    },
    Submitted,
}

impl ServerFilter {
    pub(crate) fn predicate(&self, first_param: usize) -> (String, Vec<String>) {
        match self {
            Self::ServerId(server_id) => (
                format!("\"server_id\" = ${first_param}"),
                vec![server_id.clone()],
            ),
            Self::ServerIds(server_ids) => list_predicate(server_ids, first_param),
            Self::Identifiers {
                value,
                exclude_server_id,
            } => identifier_predicate(value, exclude_server_id.as_deref(), first_param),
            Self::ApprovalStatus(status) => (
                format!("\"approval_status\" = ${first_param}"),
                vec![status.clone()],
            ),
            Self::NotDraft => (NOT_DRAFT_PREDICATE.to_owned(), Vec::new()),
            Self::SubmittedByActive { user_id } => (
                format!("\"submitted_by\" = ${first_param} AND \"approval_status\" = '{ACTIVE}'"),
                vec![user_id.clone()],
            ),
            Self::Submitted => ("\"submitted_at\" IS NOT NULL".to_owned(), Vec::new()),
        }
    }
}

fn list_predicate(server_ids: &[String], first_param: usize) -> (String, Vec<String>) {
    if server_ids.is_empty() {
        return ("FALSE".to_owned(), Vec::new());
    }
    let mut placeholders = Vec::with_capacity(server_ids.len());
    for offset in 0..server_ids.len() {
        placeholders.push(format!("${}", first_param.saturating_add(offset)));
    }
    (
        format!("\"server_id\" IN ({})", placeholders.join(", ")),
        server_ids.to_vec(),
    )
}

// both columns compare equal ignoring case; drafts cannot collide, so the NULL-or-not-draft arm applies too
fn identifier_predicate(
    value: &str,
    exclude_server_id: Option<&str>,
    first_param: usize,
) -> (String, Vec<String>) {
    let named = format!(
        "((LOWER(\"server_name\") = LOWER(${first_param}) OR LOWER(\"alias\") = LOWER(${first_param})) AND {NOT_DRAFT_PREDICATE})"
    );
    let Some(own_server_id) = exclude_server_id else {
        return (named, vec![value.to_owned()]);
    };
    let own_param = first_param.saturating_add(1);
    (
        format!("{named} AND \"server_id\" <> ${own_param}"),
        vec![value.to_owned(), own_server_id.to_owned()],
    )
}

// alias first so the reported field is deterministic; empty identifiers cannot collide
pub(crate) fn identifier_candidates<'value>(
    server_name: Option<&'value str>,
    alias: Option<&'value str>,
) -> Vec<(IdentifierField, &'value str)> {
    let mut candidates = Vec::new();
    let ordered = [
        (IdentifierField::Alias, alias),
        (IdentifierField::ServerName, server_name),
    ];
    for (field, candidate) in ordered {
        if let Some(value) = candidate
            && !value.is_empty()
        {
            candidates.push((field, value));
        }
    }
    candidates
}

#[derive(Debug, Clone)]
pub struct FindServers {
    pub filter: Option<ServerFilter>,
    pub descending: bool,
    pub take: Option<u32>,
}
