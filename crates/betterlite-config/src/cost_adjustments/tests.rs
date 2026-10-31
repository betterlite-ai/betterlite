#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use rust_decimal::Decimal;

use crate::cost_adjustments::{DiscountEntry, MarginEntry, MarginFieldEntry};
use crate::loader::ConfigLoader;

#[test]
fn loads_cost_adjustments() {
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(
        "litellm_settings:\n  cost_discount_config:\n    vertex_ai: 0.2\n  cost_margin_config:\n    vertex_ai: null\n    global: {percentage: '0.1', fixed_amount: 0.01}\n",
    )
    .unwrap();
    let settings = &config.sections.cost_adjustments;
    assert_eq!(
        settings.cost_discount_config["vertex_ai"],
        DiscountEntry::Percent(Decimal::new(2, 1))
    );
    assert_eq!(
        settings.cost_margin_config["vertex_ai"],
        MarginEntry::Unsupported,
        "a null provider entry stays an explicit unapplied entry instead of falling through to global"
    );
    assert_eq!(
        settings.cost_margin_config["global"],
        MarginEntry::Split {
            percentage: MarginFieldEntry::Number(Decimal::new(1, 1)),
            fixed_amount: MarginFieldEntry::Number(Decimal::new(1, 2)),
        }
    );
    let defaults = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("")
    .unwrap();
    let default_settings = &defaults.sections.cost_adjustments;
    assert!(default_settings.cost_discount_config.is_empty());
    assert!(default_settings.cost_margin_config.is_empty());
}

#[test]
fn loader_keeps_an_integer_fixed_amount_exact_past_f64_range() {
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(
        "litellm_settings:\n  cost_margin_config:\n    global: {fixed_amount: 9007199254740993}\n",
    )
    .unwrap();
    let settings = &config.sections.cost_adjustments;
    assert_eq!(
        settings.cost_margin_config["global"],
        MarginEntry::Split {
            percentage: MarginFieldEntry::Absent,
            fixed_amount: MarginFieldEntry::Number(
                Decimal::from_str_exact("9007199254740993").unwrap()
            ),
        },
        "an integer lexeme has no decimal point, so the loader keeps every digit past f64's 2^53 bound",
    );
}

#[test]
fn loader_keeps_quoted_text_exact_instead_of_rounding_a_float_lexeme() {
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(
        "litellm_settings:\n  cost_margin_config:\n    global: {fixed_amount: '9007199254740993.0'}\n",
    )
    .unwrap();
    let settings = &config.sections.cost_adjustments;
    assert_eq!(
        settings.cost_margin_config["global"],
        MarginEntry::Split {
            percentage: MarginFieldEntry::Absent,
            fixed_amount: MarginFieldEntry::Number(
                Decimal::from_str_exact("9007199254740993").unwrap()
            ),
        },
        "a quoted lexeme parses to the same exact decimal as the unquoted integer, with no rounding through f64",
    );
}

#[rstest]
#[case::text("a", "\"0.2\"")]
#[case::list("b", "[1]")]
#[case::object("c", "{x: 1}")]
#[case::boolean("d", "true")]
fn discount_text_list_and_object_entries_are_unsupported_instead_of_crashing(
    #[case] provider: &str,
    #[case] value: &str,
) {
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(&format!(
        "litellm_settings:\n  cost_discount_config:\n    {provider}: {value}\n"
    ))
    .unwrap();
    let settings = &config.sections.cost_adjustments;
    assert_eq!(
        settings.cost_discount_config[provider],
        DiscountEntry::Unsupported
    );
}

#[test]
fn margin_scalar_text_and_boolean_do_not_become_a_percentage() {
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("litellm_settings:\n  cost_margin_config:\n    a: \"0.5\"\n    b: true\n")
    .unwrap();
    let settings = &config.sections.cost_adjustments;
    assert_eq!(settings.cost_margin_config["a"], MarginEntry::Unsupported);
    assert_eq!(settings.cost_margin_config["b"], MarginEntry::Unsupported);
}

#[test]
fn margin_object_field_ignores_a_boolean_value_and_unrelated_keys() {
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("litellm_settings:\n  cost_margin_config:\n    global: {percentage: true, ignored: 99}\n")
    .unwrap();
    let settings = &config.sections.cost_adjustments;
    assert_eq!(
        settings.cost_margin_config["global"],
        MarginEntry::Split {
            percentage: MarginFieldEntry::Absent,
            fixed_amount: MarginFieldEntry::Absent,
        }
    );
}

#[test]
fn margin_object_field_trims_whitespace_before_converting() {
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("litellm_settings:\n  cost_margin_config:\n    global: {percentage: ' 0.25 '}\n")
    .unwrap();
    let settings = &config.sections.cost_adjustments;
    assert_eq!(
        settings.cost_margin_config["global"],
        MarginEntry::Split {
            percentage: MarginFieldEntry::Number(Decimal::new(25, 2)),
            fixed_amount: MarginFieldEntry::Absent,
        }
    );
}

#[rstest]
#[case::non_numeric_text("percentage: \"not-a-number\"")]
#[case::overflowing_text("fixed_amount: \"1e40\"")]
#[case::null_value("fixed_amount: null")]
#[case::list_value("percentage: [1]")]
#[case::nested_object_value("percentage: {x: 1}")]
fn margin_object_field_rejects_a_value_that_cannot_convert_to_a_number(#[case] field: &str) {
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(&format!(
        "litellm_settings:\n  cost_margin_config:\n    global: {{{field}}}\n"
    ))
    .unwrap();
    let settings = &config.sections.cost_adjustments;
    let MarginEntry::Split {
        percentage,
        fixed_amount,
    } = settings.cost_margin_config["global"]
    else {
        panic!(
            "expected a Split entry, got {:?}",
            settings.cost_margin_config["global"]
        );
    };
    assert!(
        percentage == MarginFieldEntry::Invalid || fixed_amount == MarginFieldEntry::Invalid,
        "a value that cannot convert to a number stays marked invalid instead of defaulting to zero"
    );
}

#[test]
fn discount_scalar_number_is_invalid_when_the_lexeme_overflows_decimal() {
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("litellm_settings:\n  cost_discount_config:\n    vertex_ai: 1e40\n")
    .unwrap();
    let settings = &config.sections.cost_adjustments;
    assert_eq!(
        settings.cost_discount_config["vertex_ai"],
        DiscountEntry::Invalid
    );
}

#[test]
fn margin_scalar_number_is_invalid_when_the_lexeme_overflows_decimal() {
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("litellm_settings:\n  cost_margin_config:\n    global: 1e40\n")
    .unwrap();
    let settings = &config.sections.cost_adjustments;
    assert_eq!(settings.cost_margin_config["global"], MarginEntry::Invalid);
}
