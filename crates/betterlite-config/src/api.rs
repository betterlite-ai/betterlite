pub mod resource_sdk_settings {
    #[path = "../resource_sdk_settings.rs"]
    mod implementation;
    #[path = "../resource_sdk_settings/tests.rs"]
    mod tests;

    pub use implementation::*;
}
