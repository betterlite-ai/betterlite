pub mod agent;
pub mod caller;
pub mod end_user;
pub mod error;
pub mod live;
pub mod org;
pub mod reads;
mod servers;
mod tests;
mod tools;
pub mod user;

// The workspace denies multiple_inherent_impl, so every level's methods live on the trait beside its reads.
use reads::CeilingReads;

// _get_allowed_mcp_servers_for_org and _get_allowed_mcp_servers_for_user answer both [] and None, which mean different things
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CeilingServers {
    NoRestriction,
    Unresolved,
    Servers(Vec<String>),
}

// the (list, restricts) pair every server ceiling answers; a later level may only cap a lower restriction
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Capped {
    pub servers: Vec<String>,
    pub restricts: bool,
}

// a keyless admitted source is denied by an unresolved org ceiling, while a key keeps its list
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CeilingSource {
    Key,
    Keyless,
}

// the ceiling steps over one set of reads; every dependency arrives through the constructor
pub struct Ceilings<Reads: CeilingReads> {
    pub reads: Reads,
}
