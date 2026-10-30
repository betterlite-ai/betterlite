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
// Mirrors price_fields::price_struct; Option<bool> keeps an absent flag distinct from false.
pub(crate) use capability_struct;
