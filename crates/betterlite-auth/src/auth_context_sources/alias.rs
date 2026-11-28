use betterlite_models::decisions::Json;

// The server supplies B302's resolver; auth cannot depend on router, which depends on auth.
pub type AliasResolver = fn(Option<&Json>, &str) -> Option<String>;
