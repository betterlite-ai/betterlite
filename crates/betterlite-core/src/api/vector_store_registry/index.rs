use betterlite_openai::api::vector_store_records::records::pydantic::LiteLlmManagedVectorStoreIndex;

#[derive(Debug, Default)]
pub struct VectorStoreIndexRegistry {
    pub indexes: Vec<LiteLlmManagedVectorStoreIndex>,
}

impl VectorStoreIndexRegistry {
    #[must_use]
    // ast-grep-ignore: no-one-line-functions
    pub const fn new(indexes: Vec<LiteLlmManagedVectorStoreIndex>) -> Self {
        Self { indexes }
    }

    #[must_use]
    pub fn find_by_name(&self, index_name: &str) -> Option<&LiteLlmManagedVectorStoreIndex> {
        for index in &self.indexes {
            if index.index_name == index_name {
                return Some(index);
            }
        }
        None
    }

    pub fn upsert(&mut self, index: LiteLlmManagedVectorStoreIndex) {
        for stored in &mut self.indexes {
            if stored.index_name == index.index_name {
                *stored = index;
                return;
            }
        }
        self.indexes.push(index);
    }

    #[must_use]
    pub fn contains_name(&self, index_name: &str) -> bool {
        for index in &self.indexes {
            if index.index_name == index_name {
                return true;
            }
        }
        false
    }
}
