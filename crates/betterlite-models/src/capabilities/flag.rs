// Mirrors price_fields::price_struct, generating Option<bool> capability flags
// so an absent flag stays distinct from an explicit false (see pitfalls).
macro_rules! capability_struct {
    (
        $(#[$meta:meta])*
        pub struct $name:ident { $($field:ident: $wire:literal),* $(,)? }
    ) => {
        $(#[$meta])*
        pub struct $name {
            $(
                #[serde(rename = $wire, skip_serializing_if = "Option::is_none")]
                pub $field: Option<bool>,
            )*
        }

        impl $name {
            pub(crate) const WIRE_NAMES: &'static [&'static str] = &[$($wire),*];
        }
    };
}
pub(crate) use capability_struct;
