#[macro_export]
macro_rules! helper_group {
    ($($operation:literal => $module:ident),* $(,)?) => {
        $(mod $module;)*

        pub mod helpers {
            pub use $crate::error::HelperError;
            pub use $crate::error::OrHelperError;
            pub use $crate::helper::{Helper, HelperResult};
        }

        pub const GROUP: $crate::group::Group = $crate::group::Group { invoke, check };

        fn invoke(
            operation: &str,
            input: &serde_json::Value,
        ) -> Option<$crate::helper::HelperResult> {
            match operation {
                $($operation => Some(<$module::Op as $crate::helper::Helper>::decode_and_run(input)),)*
                _ => None,
            }
        }

        fn check(operation: &str, input: &serde_json::Value) -> Option<Result<(), String>> {
            match operation {
                $($operation => Some(<$module::Op as $crate::helper::Helper>::check(input)),)*
                _ => None,
            }
        }
    };
}
