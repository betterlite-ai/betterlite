#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum CachedEndUser {
    // Python's _END_USER_NOT_COMPUTED object: Computed(None) is a resolved None
    #[default]
    NotComputed,
    Computed(Option<String>),
}

#[path = "end_user_caching/tests.rs"]
mod tests;
