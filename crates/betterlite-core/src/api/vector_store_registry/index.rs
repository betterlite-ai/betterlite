use betterlite_openai::api::vector_store_records::records::pydantic::LiteLlmManagedVectorStoreIndex;

#[derive(Debug, Default)]
pub struct VectorStoreIndexRegistry {
    pub indexes: Vec<LiteLlmManagedVectorStoreIndex>,
}

impl VectorStoreIndexRegistry {
    #[must_use]
    pub fn find_by_name(&self, index_name: &str) -> Option<&LiteLlmManagedVectorStoreIndex> {
        self.indexes
            .iter()
            .find(|index| index.index_name == index_name)
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
        self.indexes
            .iter()
            .any(|index| index.index_name == index_name)
    }
}
