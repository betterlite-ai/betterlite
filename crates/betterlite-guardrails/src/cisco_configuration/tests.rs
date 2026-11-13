#![cfg(test)]

use std::any::TypeId;

use betterlite_config::sections::cisco_verdict_contracts::Configured;

use crate::cisco_configuration::CiscoConfigModel;

#[test]
fn cisco_config_model_is_the_cisco_section_type() {
    assert_eq!(TypeId::of::<CiscoConfigModel>(), TypeId::of::<Configured>());
}
