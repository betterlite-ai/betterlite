use uuid::Uuid;

pub const CONFIG_GUARDRAIL_ID_NAMESPACE: Uuid =
    Uuid::from_u128(0x625f_63f4_935a_50e5_98b5_fbe7_7bab_c74a);

// The seeds are the name, then name:1, name:2 and so on; the first id the handler does not hold wins.
#[must_use]
pub fn stable_guardrail_id(guardrail_name: &str, taken: &dyn Fn(&str) -> bool) -> String {
    (0_u64..)
        .map(|occurrence| match occurrence {
            0 => guardrail_name.to_owned(),
            _ => format!("{guardrail_name}:{occurrence}"),
        })
        .map(|seed| Uuid::new_v5(&CONFIG_GUARDRAIL_ID_NAMESPACE, seed.as_bytes()).to_string())
        .skip_while(|candidate| taken(candidate))
        .next()
        .unwrap_or_default()
}
