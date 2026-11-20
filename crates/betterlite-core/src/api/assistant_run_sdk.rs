// The run SDK wrappers at assistants/main.py:1165-1365: they read the same kwargs, branch on the
// provider name, and differ only in the switches they force and the mapping they apply to failures.
pub mod call;
pub mod convert;
pub mod error;
pub mod provider;
pub mod service;

mod tests;
