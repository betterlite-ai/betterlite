use crate::cost::convert::parse;
use crate::cost::settings::{Settings, SettingsWire};

#[test]
fn cost_settings_build_the_lookup_from_the_model_cost_map() {
    let wire: SettingsWire = parse(
        r#"{"model_cost": {
        "gpt-test": {"litellm_provider": "openai", "mode": "chat", "input_cost_per_token": 0.001}
    }}"#,
    )
    .expect("the settings parse");
    let settings = Settings::build(wire).expect("the settings build");
    assert!(settings.discount.is_empty());
    assert!(settings.margin.is_empty());
}

#[test]
fn cost_settings_keep_the_adjustment_maps_empty_when_they_are_absent() {
    let wire: SettingsWire = parse(r#"{"model_cost": {}}"#).expect("the settings parse");
    let settings = Settings::build(wire).expect("the empty settings build");
    assert!(settings.discount.is_empty());
    assert!(settings.margin.is_empty());
}

#[test]
fn cost_settings_refuse_a_model_entry_that_is_not_a_price_object() {
    let wire: SettingsWire = parse(r#"{"model_cost": {"broken": 3}}"#).expect("the settings parse");
    let error = Settings::build(wire).err().expect("the model entry fails");
    assert!(error.to_string().contains("price lookup"));
}
