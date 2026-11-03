use super::{ContractError, Fact, ObjectKind};
use std::collections::BTreeMap;
use strum::EnumString;
use tokio_postgres::GenericClient;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, EnumString)]
#[strum(serialize_all = "snake_case")]
pub enum Attribute {
    Cache,
    Columns,
    Cycle,
    Default,
    Deferrable,
    Deferred,
    DeleteAction,
    Direction,
    Expression,
    Generated,
    Identity,
    Increment,
    KeyCount,
    Keys,
    Kind,
    Labels,
    Match,
    Max,
    Method,
    Min,
    NotNull,
    NullsNotDistinct,
    OwnedColumn,
    OwnedRelation,
    Position,
    Predicate,
    Start,
    Target,
    TargetColumns,
    Type,
    Typmod,
    Unique,
    UpdateAction,
    Valid,
}

const QUERIES: [(ObjectKind, &str); 6] = [
    (ObjectKind::Relation, include_str!("relations.sql")),
    (ObjectKind::Column, include_str!("columns.sql")),
    (ObjectKind::Constraint, include_str!("constraints.sql")),
    (ObjectKind::Index, include_str!("indexes.sql")),
    (ObjectKind::Sequence, include_str!("sequences.sql")),
    (ObjectKind::Enum, include_str!("enums.sql")),
];

pub(super) async fn collect<C>(
    session: &C,
    namespace: &str,
) -> Result<BTreeMap<Fact, Option<String>>, ContractError>
where
    C: GenericClient + Sync,
{
    let mut facts = BTreeMap::new();
    for (kind, query) in QUERIES {
        for row in session.query(query, &[&namespace]).await? {
            let fact = Fact {
                kind,
                relation: row.try_get(0)?,
                member: row.try_get(1)?,
                attribute: row.try_get::<_, String>(2)?.parse()?,
            };
            let value = row.try_get(3)?;
            if facts.insert(fact.clone(), value).is_some() {
                return Err(ContractError::Duplicate(fact));
            }
        }
    }
    Ok(facts)
}
