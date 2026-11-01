#![cfg(test)]

use std::any::TypeId;

use crate::cisco_configuration::CiscoConfigModel;

#[test]
fn cisco_config_model_type_id_is_distinct_from_other_types() {
    assert_ne!(TypeId::of::<CiscoConfigModel>(), TypeId::of::<u8>());
}
