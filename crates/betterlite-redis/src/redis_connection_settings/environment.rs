mod tests;

// Caller-supplied environment values, so this module never reads std::env itself.
pub use betterlite_env::EnvironmentLookup;
