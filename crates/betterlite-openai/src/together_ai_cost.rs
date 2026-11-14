mod tests;

const REGISTRY_PREFIX: &str = "together_ai/";
const BILLION_SUFFIX: u8 = b'b';
const MILLION_SUFFIX: u8 = b'm';
const UP_TO_4B: &str = "together-ai-up-to-4b";
const FROM_4_1B_TO_8B: &str = "together-ai-4.1b-8b";
const FROM_8_1B_TO_21B: &str = "together-ai-8.1b-21b";
const FROM_21_1B_TO_41B: &str = "together-ai-21.1b-41b";
const FROM_41_1B_TO_80B: &str = "together-ai-41.1b-80b";
const FROM_81_1B_TO_110B: &str = "together-ai-81.1b-110b";
const EMBEDDING_UP_TO_150M: &str = "together-ai-embedding-up-to-150m";
const EMBEDDING_151M_TO_350M: &str = "together-ai-embedding-151m-to-350m";

// The caller reads TOGETHER_AI_<N>_<UNIT> from the environment; this leaf only holds the resolved limits
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TogetherPricingTiers {
    pub up_to_4b: u32,
    pub up_to_8b: u32,
    pub up_to_21b: u32,
    pub up_to_41b: u32,
    pub up_to_80b: u32,
    pub up_to_110b: u32,
    pub embedding_150m: u32,
    pub embedding_350m: u32,
}

impl Default for TogetherPricingTiers {
    fn default() -> Self {
        Self {
            up_to_4b: 4,
            up_to_8b: 8,
            up_to_21b: 21,
            up_to_41b: 41,
            up_to_80b: 80,
            up_to_110b: 110,
            embedding_150m: 150,
            embedding_350m: 350,
        }
    }
}

// The cost map answers whether together_ai/<model> is an entry that holds input_cost_per_token
pub trait CostMap {
    fn has_price_entry(&self, key: &str) -> bool;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PricingCallType {
    Embedding,
    Aembedding,
    Other,
}

#[must_use]
pub fn model_has_registry_pricing(model: &str, cost_map: &dyn CostMap) -> bool {
    let stripped = model.strip_prefix(REGISTRY_PREFIX).unwrap_or(model);
    cost_map.has_price_entry(&format!("{REGISTRY_PREFIX}{stripped}"))
}

#[must_use]
pub fn pricing_category(
    model: &str,
    call_type: PricingCallType,
    tiers: &TogetherPricingTiers,
) -> String {
    let name = model.to_lowercase();
    match call_type {
        PricingCallType::Embedding | PricingCallType::Aembedding => {
            let limits = [
                (tiers.embedding_150m, EMBEDDING_UP_TO_150M),
                (tiers.embedding_350m, EMBEDDING_151M_TO_350M),
            ];
            parameter_count(&name, MILLION_SUFFIX)
                .and_then(|count| first_matching(count, &limits))
                .map_or(name, str::to_owned)
        }
        PricingCallType::Other => {
            let limits = [
                (tiers.up_to_4b, UP_TO_4B),
                (tiers.up_to_8b, FROM_4_1B_TO_8B),
                (tiers.up_to_21b, FROM_8_1B_TO_21B),
                (tiers.up_to_41b, FROM_21_1B_TO_41B),
                (tiers.up_to_80b, FROM_41_1B_TO_80B),
                (tiers.up_to_110b, FROM_81_1B_TO_110B),
            ];
            parameter_count(&name, BILLION_SUFFIX)
                .and_then(|count| first_matching(count, &limits))
                .map_or(name, str::to_owned)
        }
    }
}

// LiteLLM's re.search(r"(\d+b)") takes the leftmost digit run the suffix follows, so mixtral-8x7b is 7
fn parameter_count(name: &str, suffix: u8) -> Option<f64> {
    let bytes = name.as_bytes();
    for start in 0..bytes.len() {
        if !bytes[start].is_ascii_digit() {
            continue;
        }
        let mut end = start;
        while bytes.get(end).is_some_and(u8::is_ascii_digit) {
            end += 1;
        }
        if bytes.get(end) == Some(&suffix) {
            return name.get(start..end)?.parse::<f64>().ok();
        }
    }
    None
}

fn first_matching(count: f64, limits: &[(u32, &'static str)]) -> Option<&'static str> {
    limits
        .iter()
        .find_map(|(limit, category)| (count <= f64::from(*limit)).then_some(*category))
}
