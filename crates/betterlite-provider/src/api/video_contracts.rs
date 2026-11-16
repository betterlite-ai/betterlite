pub mod config;
pub mod error;
pub mod records;

// check names fix these test modules as api::video_contracts::<check>, so their declarations live under tests/
include!("video_contracts/tests/modules.rs");
