pub mod choice;
pub mod event_hooks;
pub mod payload;
pub mod records;

// Stands in for LiteLLM's CiscoAIDefenseGuardrailConfigModel, owned by the unbuilt base guardrail framework (F); B3762's guardrail impl selects it via TypeId::of::<Self>().
pub struct CiscoConfigModel;

mod tests;
