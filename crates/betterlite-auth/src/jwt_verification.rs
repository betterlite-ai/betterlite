pub mod algorithms;
pub mod claims;
pub mod decode;
pub mod errors;
pub mod keys;
pub mod normalize;
pub mod settings;
pub mod source;
pub mod userinfo;
pub mod verifier;

pub type TypedValue = betterlite_paths::paths::Document;

mod tests;
