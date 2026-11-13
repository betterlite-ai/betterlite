pub mod continuation;
pub mod create;
pub mod get_delete;
pub mod hook;
pub mod list;
pub mod wiring;

// check names fix these test modules as api::skills::<check>, and one is past the identifier limit, so their declarations live under tests/
include!("skills/tests/modules.rs");
