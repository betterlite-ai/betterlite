use indexmap::IndexMap;

// LimitedSizeOrderedDict from LiteLLM: insertion ordered, and a write past the bound drops the oldest entry
#[derive(Debug)]
pub struct LimitedSizeOrderedDict<Value> {
    pub entries: IndexMap<String, Value>,
    pub max_size: usize,
}

impl<Value> LimitedSizeOrderedDict<Value> {
    // OrderedDict.__setitem__ evicts the oldest entry first even when the key is already present
    pub fn insert(&mut self, key: String, value: Value) {
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
