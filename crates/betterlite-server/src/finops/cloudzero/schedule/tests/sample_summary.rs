#![cfg(test)]

use rust_decimal::Decimal;

use super::fixtures::{decimal, integer, row, text};
use crate::finops::cloudzero::schedule::dry_run::{DryRunError, sample_summary};
use crate::finops::cloudzero::schedule::usage_rows::UsageRowError;

mod totals {
    use super::*;

    #[test]
    fn cloudzero_schedule() {
        let rows = [
            row(&[
                ("spend", decimal("1.25")),
                ("prompt_tokens", integer(3)),
                ("completion_tokens", integer(1)),
            ]),
            row(&[
                ("spend", decimal("2.75")),
                ("prompt_tokens", integer(0)),
                ("completion_tokens", integer(2)),
            ]),
            row(&[]),
        ];
        let summary = sample_summary(&rows).expect("a summary");
        assert_eq!(summary.total_records, 3);
        assert_eq!(summary.total_cost, Decimal::new(4, 0));
        assert_eq!(summary.total_tokens, 6);
        assert_eq!(summary.unique_accounts, 0);
        assert_eq!(summary.unique_services, 0);
    }
}

mod text_spend {
    use super::*;

    #[test]
    fn cloudzero_schedule() {
        let text_spend = [row(&[("spend", text("free"))])];
        let error = sample_summary(&text_spend).expect_err("text is not a decimal");
        assert_eq!(error.to_string(), "the usage rows could not be read");
        assert!(matches!(
            error,
            DryRunError::Usage(UsageRowError::NotDecimal { field: "spend" })
        ));
    }
}

mod cost_overflow {
    use super::*;

    #[test]
    fn cloudzero_schedule() {
        let cost_overflow = [
            row(&[("spend", decimal("79228162514264337593543950335"))]),
            row(&[("spend", decimal("1"))]),
        ];
        let error = sample_summary(&cost_overflow).expect_err("a cost overflow");
        assert_eq!(
            error.to_string(),
            "the dry run total for total_cost overflowed"
        );
    }
}

mod token_overflow {
    use super::*;

    #[test]
    fn cloudzero_schedule() {
        let token_overflow = [
            row(&[
                ("prompt_tokens", integer(i64::MAX)),
                ("completion_tokens", integer(0)),
            ]),
            row(&[
                ("prompt_tokens", integer(1)),
                ("completion_tokens", integer(0)),
            ]),
        ];
        let error = sample_summary(&token_overflow).expect_err("a token overflow");
        assert_eq!(
            error.to_string(),
            "the dry run total for total_tokens overflowed"
        );
    }
}

mod row_token_overflow {
    use super::*;

    #[test]
    fn cloudzero_schedule() {
        let row_overflow = [row(&[
            ("prompt_tokens", integer(i64::MAX)),
            ("completion_tokens", integer(1)),
        ])];
        let error = sample_summary(&row_overflow).expect_err("a row token overflow");
        assert_eq!(
            error.to_string(),
            "the dry run total for total_tokens overflowed"
        );
    }
}
