#![cfg(test)]
#![expect(
    clippy::disallowed_methods,
    reason = "test fixtures are built with serde_json::json!, whose Into calls the lint table bans"
)]

mod contents;
mod corpus;
mod options;
mod prelude;
mod response_format;
mod system_instruction;
mod tools;
