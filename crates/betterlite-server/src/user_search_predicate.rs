use crate::key_fields::values::{InsensitiveContains, MatchMode};
use crate::key_query_plan::sql::SqlValue;
use crate::user_contracts::search::{UserSearchTerm, UserSearchWhere};

mod tests;

// only None and the empty string leave no predicate; a search of one space still filters on that space
#[must_use]
pub fn user_search_where(search: Option<&str>) -> Option<UserSearchWhere> {
    let term = search.filter(|value| !value.is_empty())?;
    let user_id = InsensitiveContains {
        contains: term.to_owned(),
        mode: MatchMode::Insensitive,
    };
    let user_email = InsensitiveContains {
        contains: term.to_owned(),
        mode: MatchMode::Insensitive,
    };
    let predicate = UserSearchWhere {
        or: vec![
            UserSearchTerm {
                user_id: Some(user_id),
                user_email: None,
            },
            UserSearchTerm {
                user_id: None,
                user_email: Some(user_email),
            },
        ],
    };
    Some(predicate)
}

// Prisma renders contains as ILIKE with the pattern unescaped, so % _ and \ keep their LIKE meaning
#[must_use]
pub fn user_term_sql(term: &UserSearchTerm, params: &mut Vec<SqlValue>) -> String {
    let mut parts = Vec::new();
    if let Some(filter) = &term.user_id {
        parts.push(column_sql("user_id", filter, params));
    }
    if let Some(filter) = &term.user_email {
        parts.push(column_sql("user_email", filter, params));
    }
    if parts.is_empty() {
        return String::from("TRUE");
    }
    parts.join(" AND ")
}

// Prisma matches nothing for an empty OR list, and a term on neither column matches everything
#[must_use]
pub fn user_search_sql(predicate: &UserSearchWhere, params: &mut Vec<SqlValue>) -> String {
    if predicate.or.is_empty() {
        return String::from("FALSE");
    }
    let terms = predicate
        .or
        .iter()
        .map(|term| user_term_sql(term, params))
        .collect::<Vec<_>>()
        .join(" OR ");
    format!("({terms})")
}

// the membership table has its own user_id, so the filter stays a subquery over the outer column
#[must_use]
pub fn organization_membership_sql(
    organization_ids: &[String],
    params: &mut Vec<SqlValue>,
) -> String {
    let ids = bind(params, SqlValue::TextList(organization_ids.to_vec()));
    format!(
        "\"user_id\" IN (SELECT \"user_id\" FROM \"LiteLLM_OrganizationMembership\" WHERE \"organization_id\" = ANY({ids}))"
    )
}

fn column_sql(column: &str, filter: &InsensitiveContains, params: &mut Vec<SqlValue>) -> String {
    let pattern = format!("%{}%", filter.contains);
    let placeholder = bind(params, SqlValue::Text(pattern));
    format!("\"{column}\" ILIKE {placeholder}")
}

fn bind(params: &mut Vec<SqlValue>, value: SqlValue) -> String {
    params.push(value);
    let index = params.len();
    format!("${index}")
}
