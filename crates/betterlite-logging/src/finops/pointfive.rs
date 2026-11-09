pub mod error;
pub mod payload;
pub mod types;
pub mod upload;

// the task's nextest filters name these modules finops::pointfive::<name>_tests_behavior
#[path = "pointfive/tests/payload_tests_behavior.rs"]
mod payload_tests_behavior;
#[path = "pointfive/tests/types_tests_behavior.rs"]
mod types_tests_behavior;
#[path = "pointfive/tests/upload_tests_behavior.rs"]
mod upload_tests_behavior;
