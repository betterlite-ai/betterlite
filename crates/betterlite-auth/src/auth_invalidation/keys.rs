// the auth_checks.py key formats the refresh and eviction helpers own; every other key comes from CacheKey
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvalidationKey<'key> {
    TeamId(&'key str),
    TeamAlias(&'key str),
    AccessGroupId(&'key str),
    SpendCounter {
        user_id: &'key str,
        team_id: &'key str,
    },
    SpendFloor {
        user_id: &'key str,
        team_id: &'key str,
    },
}

impl InvalidationKey<'_> {
    #[must_use]
    pub fn render(&self) -> String {
        match self {
            Self::TeamId(team_id) => format!("team_id:{team_id}"),
            Self::TeamAlias(alias) => format!("team_alias:{alias}"),
            Self::AccessGroupId(access_group_id) => format!("access_group_id:{access_group_id}"),
            Self::SpendCounter { user_id, team_id } => {
                format!("spend:team_member:{user_id}:{team_id}")
            }
            Self::SpendFloor { user_id, team_id } => {
                format!("spend_db_floor:spend:team_member:{user_id}:{team_id}")
            }
        }
    }
}
