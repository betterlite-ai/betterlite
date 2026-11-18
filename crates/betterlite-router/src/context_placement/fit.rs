use std::collections::BTreeMap;

use crate::complexity_math::window_holds;

use super::facts::WindowFacts;

// _RequestContextFit: the window facts of every candidate group, the real count when the byte bound needed one
#[derive(Clone, Debug, PartialEq)]
pub struct ContextFit {
    pub facts: BTreeMap<String, WindowFacts>,
    pub needed: Option<u64>,
    pub buffer: f64,
}

impl ContextFit {
    // _RequestContextFit.accepts: a group with no declared window accepts, as does a fit with no count
    #[must_use]
    pub fn accepts(&self, model: &str) -> bool {
        self.needed.map_or(true, |needed| {
            window_holds(
                window_facts(&self.facts, model).window,
                Some(needed),
                self.buffer,
            )
        })
    }
}

pub(super) fn window_facts(facts: &BTreeMap<String, WindowFacts>, group: &str) -> WindowFacts {
    let Some(found) = facts.get(group) else {
        return WindowFacts::UNKNOWN;
    };
    *found
}
