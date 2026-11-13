pub mod claims;
pub mod decode;
pub mod handler;
pub mod identity;
pub mod orgs;
pub mod settings;
pub mod teams;
pub mod trust;

use betterlite_paths::paths::Document;

// the typed JSON document B213 navigates; the dependency holds the one expect for untyped operator JSON
pub type TypedValue = Document;

mod tests;
