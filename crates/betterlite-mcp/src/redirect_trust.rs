pub mod loopback;
pub mod origins;
pub mod shape;
pub mod trusted;

mod tests;

// The ops allowlists the redirect trust policies walk; callers build one value from config, never from env.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RedirectAllowlists {
    pub origins: Vec<String>,
    pub native_callbacks: Vec<String>,
}
