use std::collections::BTreeMap;

use percent_encoding::percent_decode_str;
use rust_decimal::Decimal;
use serde::Serialize;
use thiserror::Error;
use tokio_postgres::types::Json;

use crate::key_user_writes::retry::WriteClock;
use crate::postgres_store::session::DbSession;

// TEAM_ADVISORY_LOCK_SQL: the per-team lock the team endpoints hold while they change a roster, so the roster check cannot interleave with them
pub const TEAM_ADVISORY_LOCK_SQL: &str =
    "SELECT pg_advisory_xact_lock(hashtext($1)) IS NULL AS locked";

// A missing membership is created only while the user is still on the roster, so a flush landing after a removal never recreates the member.
pub const TEAM_MEMBER_SPEND_SQL: &str = r#"INSERT INTO "LiteLLM_TeamMembership" (user_id, team_id, spend, total_spend)
SELECT member.user_id, member.team_id, member.cost::double precision, member.cost::double precision
FROM jsonb_to_recordset($1::jsonb) AS member(user_id text, team_id text, cost numeric)
WHERE EXISTS (
    SELECT 1 FROM "LiteLLM_TeamTable" t
    WHERE t.team_id = member.team_id
      AND t.members_with_roles @> jsonb_build_array(jsonb_build_object('user_id', member.user_id))
)
   OR EXISTS (
    SELECT 1 FROM "LiteLLM_TeamMembership" m
    WHERE m.user_id = member.user_id AND m.team_id = member.team_id
)
ON CONFLICT (user_id, team_id) DO UPDATE
SET spend = "LiteLLM_TeamMembership".spend + EXCLUDED.spend,
    total_spend = "LiteLLM_TeamMembership".total_spend + EXCLUDED.total_spend"#;

pub const ORG_MEMBER_SPEND_SQL: &str = r#"UPDATE "LiteLLM_OrganizationMembership" SET spend = spend + $3::numeric::double precision, updated_at = $4 WHERE organization_id = $1 AND user_id = $2"#;

#[derive(Debug, Error, PartialEq, Eq)]
#[error("member key {key:?} does not hold the four `::` components of an id pair")]
pub struct MemberKeyError {
    pub key: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TeamMemberKey<'key> {
    pub team_id: &'key str,
    pub user_id: &'key str,
}

// key is "team_id::<value>::user_id::<value>", read raw at its second and fourth components as the source's split does
pub fn team_member_key(key: &str) -> Result<TeamMemberKey<'_>, MemberKeyError> {
    let mut parts = key.split("::");
    let (Some(team_id), Some(user_id)) = (parts.nth(1), parts.nth(1)) else {
        return Err(MemberKeyError {
            key: key.to_owned(),
        });
    };
    Ok(TeamMemberKey { team_id, user_id })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrgMemberSpend {
    pub organization_id: String,
    pub user_id: String,
    pub cost: Decimal,
}

// _, quoted_org_id, _, quoted_user_id = key.split("::"): exactly four components, each id decoded the way unquote does
pub fn org_member_rows(
    increments: &BTreeMap<String, Decimal>,
) -> Result<Vec<OrgMemberSpend>, MemberKeyError> {
    let mut rows = Vec::with_capacity(increments.len());
    for (key, cost) in increments {
        let parts: Vec<&str> = key.split("::").collect();
        let [_org_label, quoted_org_id, _user_label, quoted_user_id] = parts.as_slice() else {
            return Err(MemberKeyError { key: key.clone() });
        };
        rows.push(OrgMemberSpend {
            organization_id: percent_decode_str(quoted_org_id)
                .decode_utf8_lossy()
                .into_owned(),
            user_id: percent_decode_str(quoted_user_id)
                .decode_utf8_lossy()
                .into_owned(),
            cost: *cost,
        });
    }
    Ok(rows)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemberSpendRow<'key> {
    pub user_id: &'key str,
    pub team_id: &'key str,
    // jsonb carries the exact decimal, so no batch types a later one's parameter
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub cost: Decimal,
}

// The rows one team membership transaction writes, sorted, and the teams they lock before the upsert.
pub struct TeamMemberWrite<'key> {
    pub rows: Vec<MemberSpendRow<'key>>,
    pub teams: Vec<&'key str>,
}

// The source sorts (team_id, user_id, cost) tuples, which is not the raw key order once an id holds a byte below ':'.
pub fn team_member_rows(
    increments: &BTreeMap<String, Decimal>,
) -> Result<TeamMemberWrite<'_>, MemberKeyError> {
    let mut rows = Vec::with_capacity(increments.len());
    for (key, cost) in increments {
        let member = team_member_key(key)?;
        rows.push(MemberSpendRow {
            user_id: member.user_id,
            team_id: member.team_id,
            cost: *cost,
        });
    }
    rows.sort_by(|left, right| {
        (left.team_id, left.user_id, left.cost).cmp(&(right.team_id, right.user_id, right.cost))
    });
    let teams = team_lock_order(&rows);
    Ok(TeamMemberWrite { rows, teams })
}

// Locks are taken once per team in sorted team order, like the team endpoints take them.
#[must_use]
pub fn team_lock_order<'key>(rows: &[MemberSpendRow<'key>]) -> Vec<&'key str> {
    let mut team_ids: Vec<&str> = rows.iter().map(|row| row.team_id).collect();
    team_ids.dedup();
    team_ids
}

// What one membership table's transaction writes.
pub enum MemberSpendWrite<'rows> {
    Organization(Vec<OrgMemberSpend>),
    Team(TeamMemberWrite<'rows>),
}

// _write_team_member_spend and the org member updates: the roster-checked upsert behind the sorted per-team advisory locks, and one increment per existing organization membership, each statement stamping its own now into @updatedAt.
pub(super) async fn write_member_spend(
    transaction: &DbSession<'_>,
    write: MemberSpendWrite<'_>,
    clock: &dyn WriteClock,
) -> Result<(), tokio_postgres::Error> {
    match write {
        MemberSpendWrite::Team(members) => {
            for team_id in members.teams {
                transaction
                    .execute(TEAM_ADVISORY_LOCK_SQL, &[&team_id])
                    .await?;
            }
            transaction
                .execute(TEAM_MEMBER_SPEND_SQL, &[&Json(members.rows.as_slice())])
                .await?;
        }
        MemberSpendWrite::Organization(rows) => {
            let statement = transaction.prepare(ORG_MEMBER_SPEND_SQL).await?;
            for row in rows {
                let at = clock.now().naive_utc();
                transaction
                    .execute(
                        &statement,
                        &[&row.organization_id, &row.user_id, &row.cost, &at],
                    )
                    .await?;
            }
        }
    }
    Ok(())
}

mod tests;
