macro_rules! nullable_record {
    (($($derive:path),* $(,)?) $(#[$meta:meta])* $vis:vis struct $name:ident { $($(#[$field_meta:meta])* $field:ident: $ty:ty),* $(,)? } $(secret { $($secret:ident),* $(,)? })?) => {
        $(#[$meta])*
        #[derive($($derive),*)]
        $vis struct $name {
            $(
                $(#[$field_meta])*
                #[serde(
                    default,
                    skip_serializing_if = "Option::is_none",
                    with = "serde_with::rust::double_option"
                )]
                pub $field: Option<Option<$ty>>,
            )*
            $($(
                #[serde(default, skip_serializing_if = "Option::is_none", serialize_with = "serde_with::As::<Option<Option<serde_with::FromIntoRef<crate::key_fields::values::ExposedSecret>>>>::serialize", deserialize_with = "serde_with::rust::double_option::deserialize")]
                pub $secret: Option<Option<secrecy::SecretString>>,
            )*)?
        }
    };
}

pub mod adapters;
pub mod base;
pub mod broadcast;
pub mod bulk;
pub mod decode;
pub mod error;
pub mod policy;
pub mod regenerate;
pub mod request;
pub mod response;
pub mod shared;
pub mod update;
pub mod values;
pub mod windows;

mod defaults;
#[cfg(test)]
mod tests;
