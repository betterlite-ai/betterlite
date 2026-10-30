#![cfg(test)]

mod s3_v2_params {
    use super::super::{PartitionGranularityCache, S3ConstructorParameters, resolve_s3_parameters};
    use crate::s3_transport::option_value::OptionValue;
    use betterlite_config::loader::EnvironmentLookup;
    use serde_json::json;

    #[derive(Default)]
    struct Environment;

    impl EnvironmentLookup for Environment {
        fn lookup(&self, _name: &str) -> Option<String> {
            None
        }
    }

    #[test]
    fn s3_params_defaults_and_override_do_not_mutate_source() {
        let source = json!({"s3_bucket_name": "global", "s3_max_queue_size": 0});
        let source = source.as_object().expect("object");
        let before = source.clone();
        let parameters = resolve_s3_parameters(
            &S3ConstructorParameters::default(),
            source,
            None,
            &Environment,
        );
        assert_eq!(parameters.bucket_name.as_deref(), Some("global"));
        assert_eq!(parameters.max_queue_size, 50_000);
        assert_eq!(*source, before);
    }

    #[test]
    fn s3_params_partition_cache_uses_raw_value() {
        let mut cache = PartitionGranularityCache::default();
        let environment = Environment;
        let day = OptionValue::Text("day".to_owned());
        let hour = OptionValue::Text("hour".to_owned());
        assert_eq!(
            cache.resolve(Some(&day), &environment),
            crate::s3_transport::options::PartitionGranularity::Day
        );
        assert_eq!(
            cache.resolve(Some(&hour), &environment),
            crate::s3_transport::options::PartitionGranularity::Hour
        );
    }
}
