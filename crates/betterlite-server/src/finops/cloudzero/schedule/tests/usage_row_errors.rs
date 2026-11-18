#![cfg(test)]

use betterlite_db::budget_permission_rows::JsonData;

use super::fixtures::{decimal, integer, row, text};
use crate::finops::cloudzero::schedule::usage_rows::{UsageRowError, usage_record};

mod not_text;

mod not_integer;

mod not_decimal;

mod out_of_range;
