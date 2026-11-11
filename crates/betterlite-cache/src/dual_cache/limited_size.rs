use indexmap::IndexMap;

// LimitedSizeOrderedDict from LiteLLM: insertion ordered, and a write past the bound drops the oldest entry
#[derive(Debug)]
pub(crate) struct LimitedSizeOrderedDict<Value> {
    pub(crate) entries: IndexMap<String, Value>,
    pub(crate) max_size: usize,
}

impl<Value> LimitedSizeOrderedDict<Value> {
    // OrderedDict.__setitem__ evicts the oldest entry first even when the key is already present
    pub(crate) fn insert(&mut self, key: String, value: Value) {
        if self.max_size == 0 {
            return;
        }
        while self.entries.len() >= self.max_size {
            let Some(oldest) = self.entries.keys().next().cloned() else {
                return;
            };
            self.entries.shift_remove(&oldest);
        }
        self.entries.insert(key, value);
    }
}
