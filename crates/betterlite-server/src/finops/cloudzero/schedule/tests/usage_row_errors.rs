#![cfg(test)]

use betterlite_db::budget_permission_rows::JsonData;

use super::fixtures::{decimal, integer, row, text};
use crate::finops::cloudzero::schedule::usage_rows::{UsageRowError, usage_records};

mod not_text {
    use super::*;

    #[test]
    fn cloudzero_schedule() {
        let not_text =
            usage_records(&[row(&[("model", integer(1))])]).expect_err("a number is not text");
        assert_eq!(
            not_text.to_string(),
            "the usage row field 'model' is not text"
        );
        assert!(matches!(
            not_text,
            UsageRowError::NotText { field: "model" }
        ));

        let wrong_bool = usage_records(&[row(&[("date", JsonData::Bool(true))])])
            .expect_err("a bool is not text");
        assert!(matches!(
            wrong_bool,
            UsageRowError::NotText { field: "date" }
        ));
    }
}

mod not_integer {
    use super::*;

    #[test]
    fn cloudzero_schedule() {
        let not_integer = usage_records(&[row(&[("prompt_tokens", text("ten"))])])
            .expect_err("text is not an integer");
        assert_eq!(
            not_integer.to_string(),
            "the usage row field 'prompt_tokens' is not an integer"
        );
        assert!(matches!(
            not_integer,
            UsageRowError::NotInteger {
                field: "prompt_tokens"
            }
        ));

        let fractional = usage_records(&[row(&[("api_requests", decimal("1.5"))])])
            .expect_err("a fraction is not an integer");
        assert!(matches!(
            fractional,
            UsageRowError::NotInteger {
                field: "api_requests"
            }
        ));
    }
}

mod not_decimal {
    use super::*;

    #[test]
    fn cloudzero_schedule() {
        let not_decimal =
            usage_records(&[row(&[("spend", text("1.25"))])]).expect_err("text is not a decimal");
        assert_eq!(
            not_decimal.to_string(),
            "the usage row field 'spend' is not a decimal number"
        );
        assert!(matches!(
            not_decimal,
            UsageRowError::NotDecimal { field: "spend" }
        ));
    }
}

mod out_of_range {
    use super::*;

    #[test]
    fn cloudzero_schedule() {
        let out_of_range = usage_records(&[row(&[("spend", decimal("1e400"))])])
            .expect_err("an oversized number is refused");
        assert_eq!(
            out_of_range.to_string(),
            "the usage row field 'spend' is not a valid decimal number"
        );
        assert!(matches!(
            out_of_range,
            UsageRowError::Decimal { field: "spend", .. }
        ));
    }
}
