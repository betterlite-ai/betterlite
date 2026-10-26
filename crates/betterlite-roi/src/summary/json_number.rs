use std::str::FromStr as _;

use rust_decimal::Decimal;
use serde::Serializer;
use serde::ser::Error as _;
use serde_with::SerializeAs;

// Decimal::to_f64 rounds twice, so 2/3 would print 0.6666666666666667 where Python prints 0.6666666666666666
pub(super) struct NearestFloat;

impl SerializeAs<Decimal> for NearestFloat {
    fn serialize_as<S>(source: &Decimal, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let number = f64::from_str(&source.to_string()).map_err(S::Error::custom)?;
        serializer.serialize_f64(number)
    }
}
