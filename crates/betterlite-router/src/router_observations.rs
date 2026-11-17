pub mod counter;
pub mod failure;
pub mod keys;
pub mod success;
pub mod track;
pub mod usage;

mod tests;

use crate::cooldown_decisions::failure::Seconds;

use self::counter::CounterSink;
use self::failure::CooldownSink;

// LiteLLM keeps the counters, the cooldown cache and the router's default cooldown time together on the Router
pub struct RouterObservations<'a> {
    pub counters: &'a dyn CounterSink,
    pub cooldowns: &'a dyn CooldownSink,
    pub cooldown_time: Seconds,
}
