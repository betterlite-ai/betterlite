pub const USER_NO_MCP_PERMISSION: &str = "__user_no_mcp_permission__";
pub const TAG_REGISTRY_OVERFLOW: &str = "__tag_registry_overflow__";
pub const GROUP_REGISTRY_OVERFLOW: &str = "__model_access_group_registry_overflow__";
pub const END_USER_RESTRICTED_OVERFLOW: &str = "__end_user_restricted_registry_overflow__";
pub const NO_TEAM_MEMBERSHIP: &str = "__no_team_membership__";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Partition {
    Ordinary,
    KeyObject,
}

// LiteLLM's _HASHED_TOKEN_CACHE_KEY is a fullmatch on exactly 64 lowercase ascii hex characters
const KEY_OBJECT_HEX_DIGITS: usize = 64;

#[must_use]
pub fn partition(key: &str) -> Partition {
    let lower_hex = key.len() == KEY_OBJECT_HEX_DIGITS
        && key
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if lower_hex {
        Partition::KeyObject
    } else {
        Partition::Ordinary
    }
}

// one literal per cache protocol key, so no two modules can drift on the format
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheKey<'key> {
    UserObjectPermissionId(&'key str),
    ObjectPermissionId(&'key str),
    Tag(&'key str),
    TagRegistry,
    ModelAccessGroup(&'key str),
    ModelAccessGroupRegistry,
    ModelAccessGroupSpend(&'key str),
    ProjectId(&'key str),
    ProjectSpend(&'key str),
    EndUserId(&'key str),
    EndUserRestrictedRegistry,
    // the admission reader orders team before user, the reservation writer user before team
    TeamMembershipAuth {
        team_id: &'key str,
        user_id: &'key str,
    },
    TeamMembershipReservation {
        user_id: &'key str,
        team_id: &'key str,
    },
}

impl CacheKey<'_> {
    #[must_use]
    pub fn render(&self) -> String {
        match self {
            Self::UserObjectPermissionId(user_id) => format!("user_object_permission_id:{user_id}"),
            Self::ObjectPermissionId(id) => format!("object_permission_id:{id}"),
            Self::Tag(name) => format!("tag:{name}"),
            Self::TagRegistry => "tag_registry".to_owned(),
            Self::ModelAccessGroup(name) => format!("model_access_group:{name}"),
            Self::ModelAccessGroupRegistry => "model_access_group_registry".to_owned(),
            Self::ModelAccessGroupSpend(name) => format!("spend:model_access_group:{name}"),
            Self::ProjectId(id) => format!("project_id:{id}"),
            Self::ProjectSpend(id) => format!("spend:project:{id}"),
            Self::EndUserId(id) => format!("end_user_id:{id}"),
            Self::EndUserRestrictedRegistry => "end_user_restricted_registry".to_owned(),
            Self::TeamMembershipAuth { team_id, user_id } => format!("{team_id}_{user_id}"),
            Self::TeamMembershipReservation { user_id, team_id } => {
                format!("team_membership:{user_id}:{team_id}")
            }
        }
    }
}
