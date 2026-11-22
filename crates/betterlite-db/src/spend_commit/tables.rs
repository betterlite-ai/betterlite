use std::collections::BTreeMap;

use rust_decimal::Decimal;

use super::policy::SpendFailure;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SpendTable {
    UserList,
    EndUserList,
    KeyList,
    TeamList,
    TeamMemberList,
    OrgList,
    OrgMemberList,
    ProjectList,
    TagList,
    ModelAccessGroupList,
    AgentList,
}

impl SpendTable {
    // _SPEND_TABLE_COMMIT_ORDER: a failure keeps every table after the one in flight
    pub const COMMIT_ORDER: [Self; 11] = [
        Self::UserList,
        Self::EndUserList,
        Self::KeyList,
        Self::TeamList,
        Self::TeamMemberList,
        Self::OrgList,
        Self::OrgMemberList,
        Self::ProjectList,
        Self::TagList,
        Self::ModelAccessGroupList,
        Self::AgentList,
    ];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::UserList => "user_list_transactions",
            Self::EndUserList => "end_user_list_transactions",
            Self::KeyList => "key_list_transactions",
            Self::TeamList => "team_list_transactions",
            Self::TeamMemberList => "team_member_list_transactions",
            Self::OrgList => "org_list_transactions",
            Self::OrgMemberList => "org_member_list_transactions",
            Self::ProjectList => "project_list_transactions",
            Self::TagList => "tag_list_transactions",
            Self::ModelAccessGroupList => "model_access_group_list_transactions",
            Self::AgentList => "agent_list_transactions",
        }
    }
}

pub type SpendTableTransactions = BTreeMap<SpendTable, Option<BTreeMap<String, Decimal>>>;

// _spend_tables_left_to_send: the committed prefix clears, an unsafe failure also clears the table in flight, and nothing left is None.
#[must_use]
pub fn spend_tables_left_to_send(
    transactions: &SpendTableTransactions,
    committed: &[SpendTable],
    failure: SpendFailure,
) -> Option<SpendTableTransactions> {
    let dropped = SpendTable::COMMIT_ORDER
        .get(committed.len())
        .copied()
        .filter(|_in_flight| !failure.is_requeue_safe());
    if let Some(table) = dropped {
        tracing::error!(
            table = table.name(),
            "Spend tracking - dropped unsafe increments"
        );
    }
    let mut remaining = SpendTableTransactions::new();
    let mut any_left = false;
    for (table, increment) in transactions {
        let keep = !committed.contains(table) && Some(*table) != dropped;
        let value = if keep { increment.clone() } else { None };
        any_left |= value.as_ref().is_some_and(|rows| !rows.is_empty());
        remaining.insert(*table, value);
    }
    any_left.then_some(remaining)
}
