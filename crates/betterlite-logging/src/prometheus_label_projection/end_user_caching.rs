// Python's _END_USER_NOT_COMPUTED object: NotComputed is never resolved, Computed(None) is a resolved None
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum CachedEndUser {
    #[default]
    NotComputed,
    Computed(Option<String>),
}

#[path = "end_user_caching/tests.rs"]
mod tests;
