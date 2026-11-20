use std::str::{self, Utf8Error};

use thiserror::Error;

pub const END_USER_COUNTER_PREFIX: &str = "spend:end_user:";

const KEY_PREFIX: &str = "spend:key:";
const TEAM_MEMBER_PREFIX: &str = "spend:team_member:";
const TEAM_PREFIX: &str = "spend:team:";
const USER_PREFIX: &str = "spend:user:";
const ORGANIZATION_PREFIX: &str = "spend:org:";
const PROJECT_PREFIX: &str = "spend:project:";
const WINDOW_SEPARATOR: &str = ":window:";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowEntity {
    Key,
    Team,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpendCounterKey<'key> {
    Key(&'key str),
    TeamMember {
        user_id: &'key str,
        team_id: &'key str,
    },
    Team(&'key str),
    User(&'key str),
    Organization(&'key str),
    Project(&'key str),
    EndUser(&'key str),
    Window {
        entity: WindowEntity,
        duration: &'key str,
    },
    Unknown(&'key str),
}

// B199's duration parser decides which :window: suffixes are per-window counters, so the db crate takes it as a dependency instead of copying the grammar
pub trait WindowDurationValidity: Send + Sync {
    fn is_valid(&self, spec: &str) -> bool;
}

#[derive(Debug, Error)]
pub enum CounterKeyError {
    #[error("a spend counter key is not UTF-8")]
    NotText(#[source] Utf8Error),
}

// The prefixes, the last-colon split and the window suffix all follow SpendCounterReseed._counter_row and _is_key_or_team_window_counter
pub fn decode_spend_counter_key<'key>(
    bytes: &'key [u8],
    durations: &dyn WindowDurationValidity,
) -> Result<SpendCounterKey<'key>, CounterKeyError> {
    let text = str::from_utf8(bytes).map_err(CounterKeyError::NotText)?;
    Ok(decode_counter_text(text, durations))
}

fn decode_counter_text<'key>(
    text: &'key str,
    durations: &dyn WindowDurationValidity,
) -> SpendCounterKey<'key> {
    if let Some(token) = text.strip_prefix(KEY_PREFIX) {
        return window_or_primary(WindowEntity::Key, token, durations);
    }
    if let Some(suffix) = text.strip_prefix(TEAM_MEMBER_PREFIX) {
        return match suffix.rsplit_once(':') {
            Some((user_id, team_id)) => SpendCounterKey::TeamMember { user_id, team_id },
            None => SpendCounterKey::Unknown(text),
        };
    }
    if let Some(team_id) = text.strip_prefix(TEAM_PREFIX) {
        return window_or_primary(WindowEntity::Team, team_id, durations);
    }
    if let Some(user_id) = text.strip_prefix(USER_PREFIX) {
        return SpendCounterKey::User(user_id);
    }
    if let Some(organization_id) = text.strip_prefix(ORGANIZATION_PREFIX) {
        return SpendCounterKey::Organization(organization_id);
    }
    if let Some(project_id) = text.strip_prefix(PROJECT_PREFIX) {
        return SpendCounterKey::Project(project_id);
    }
    if let Some(user_id) = text.strip_prefix(END_USER_COUNTER_PREFIX) {
        return SpendCounterKey::EndUser(user_id);
    }
    SpendCounterKey::Unknown(text)
}

// A key/team ID keeps every delimiter literally unless the last :window: suffix parses, which is what makes it a per-window counter with no primary row
fn window_or_primary<'key>(
    entity: WindowEntity,
    rest: &'key str,
    durations: &dyn WindowDurationValidity,
) -> SpendCounterKey<'key> {
    if let Some((_head, duration)) = rest.rsplit_once(WINDOW_SEPARATOR)
        && !duration.is_empty()
        && durations.is_valid(duration)
    {
        return SpendCounterKey::Window { entity, duration };
    }
    match entity {
        WindowEntity::Key => SpendCounterKey::Key(rest),
        WindowEntity::Team => SpendCounterKey::Team(rest),
    }
}
