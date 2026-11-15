use uuid::Uuid;

const NAMESPACE: Uuid = Uuid::from_u128(0x625f_63f4_935a_50e5_98b5_fbe7_7babc74a);

#[must_use]
pub fn stable_guardrail_id(guardrail_name: &str, taken: &dyn Fn(&str) -> bool) -> String {
    let mut occurrence = 0_u64;
    loop {
        let seed = match occurrence {
            0 => guardrail_name.to_owned(),
            _ => format!("{guardrail_name}:{occurrence}"),
        };
        let candidate = Uuid::new_v5(&NAMESPACE, seed.as_bytes()).to_string();
        if !taken(&candidate) {
            return candidate;
        }
        occurrence = occurrence.saturating_add(1);
    }
}
