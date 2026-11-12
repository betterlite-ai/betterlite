#![cfg(test)]

use std::ffi::CString;

use proptest::arbitrary::any;
use proptest::collection::{btree_map, vec as arbitrary_vec};
use proptest::prop_oneof;
use proptest::proptest;
use proptest::strategy::{Just, Strategy};
use pyo3::types::{
    PyAnyMethods as _, PyDict, PyList, PyListMethods as _, PyTuple, PyTypeMethods as _,
};
use pyo3::{Py, PyAny, Python};
use serde_json::{Map, Number, Value};

use super::{MAX_DEPTH, ValueError, from_object, to_object};
use crate::host::{HostConfig, PythonHost};

fn start_host() -> PythonHost {
    PythonHost::start(&HostConfig::default()).unwrap()
}

fn evaluate(py: Python<'_>, source: &str) -> Py<PyAny> {
    let code = CString::new(source).unwrap();
    py.eval(&code, None, None).unwrap().unbind()
}

fn nested_list(py: Python<'_>, depth: usize) -> Py<PyAny> {
    let mut value = py.None();
    for _ in 0..depth {
        let list = PyList::empty(py);
        list.append(value.bind(py)).unwrap();
        value = list.into_any().unbind();
    }
    value
}

fn nested_tuple(py: Python<'_>, depth: usize) -> Py<PyAny> {
    let mut value = py.None();
    for _ in 0..depth {
        let tuple = PyTuple::new(py, [value.bind(py)]).unwrap();
        value = tuple.into_any().unbind();
    }
    value
}

fn nested_dict(py: Python<'_>, depth: usize) -> Py<PyAny> {
    let mut value = py.None();
    for _ in 0..depth {
        let dict = PyDict::new(py);
        dict.set_item("nest", value.bind(py)).unwrap();
        value = dict.into_any().unbind();
    }
    value
}

fn nested_array(depth: usize) -> Value {
    let mut value = Value::Null;
    for _ in 0..depth {
        value = Value::Array(vec![value]);
    }
    value
}

fn nested_object(depth: usize) -> Value {
    let mut value = Value::Null;
    for _ in 0..depth {
        let mut entry = Map::new();
        entry.insert("nest".to_owned(), value);
        value = Value::Object(entry);
    }
    value
}

#[rstest::rstest]
#[case::null(Value::Null, "NoneType")]
#[case::boolean(Value::Bool(true), "bool")]
#[case::negative_integer(Value::from(-7_i64), "int")]
#[case::large_unsigned(Value::from(u64::MAX), "int")]
#[case::float(Value::from(1.5_f64), "float")]
#[case::text(Value::from("hello"), "str")]
#[case::list(Value::Array(vec![Value::from(1_i64)]), "list")]
#[case::dict(Value::Object(Map::new()), "dict")]
fn to_python_builds_the_matching_python_type(#[case] value: Value, #[case] expected: &str) {
    let host = start_host();
    host.attach(|py| {
        let object = to_object(py, &value).unwrap();
        let name = object
            .get_type()
            .name()
            .unwrap()
            .extract::<String>()
            .unwrap();
        assert_eq!(name, expected);
    });
}

#[rstest::rstest]
#[case::null(Value::Null)]
#[case::boolean(Value::Bool(false))]
#[case::negative_integer(Value::from(-7_i64))]
#[case::large_unsigned(Value::from(u64::MAX))]
#[case::float(Value::from(1.5_f64))]
#[case::text(Value::from("hello"))]
#[case::nested(
    Value::Array(vec![
        Value::from(1_i64),
        Value::Array(vec![Value::Bool(true), Value::from("two")]),
        Value::Object(Map::from_iter([("three".to_owned(), Value::from(3.5_f64))])),
    ])
)]
fn to_python_and_from_python_round_trip(#[case] value: Value) {
    let host = start_host();
    host.attach(|py| {
        let object = to_object(py, &value).unwrap();
        let back = from_object(&object).unwrap();
        assert_eq!(back, value);
    });
}

#[test]
fn from_python_reads_a_tuple_as_a_list() {
    let host = start_host();
    host.attach(|py| {
        let object = evaluate(py, "(1, 'two')");
        assert_eq!(
            from_object(object.bind(py)).unwrap(),
            Value::Array(vec![Value::from(1_i64), Value::from("two")])
        );
    });
}

#[test]
fn from_python_reads_true_as_a_boolean_not_an_integer() {
    let host = start_host();
    host.attach(|py| {
        let object = evaluate(py, "True");
        assert_eq!(from_object(object.bind(py)).unwrap(), Value::Bool(true));
    });
}

#[rstest::rstest]
#[case::bytes("b'raw'", "bytes")]
#[case::bytearray("bytearray(b'raw')", "bytearray")]
#[case::set("{1, 2}", "set")]
#[case::complex("1j", "complex")]
#[case::range("range(3)", "range")]
fn from_python_refuses_a_type_without_a_json_form(#[case] source: &str, #[case] expected: &str) {
    let host = start_host();
    host.attach(|py| {
        let object = evaluate(py, source);
        assert_eq!(
            from_object(object.bind(py)),
            Err(ValueError::Unsupported {
                type_name: expected.to_owned()
            })
        );
    });
}

#[test]
fn from_python_refuses_a_non_string_dict_key() {
    let host = start_host();
    host.attach(|py| {
        let object = evaluate(py, "{1: 'one'}");
        assert_eq!(
            from_object(object.bind(py)),
            Err(ValueError::Unsupported {
                type_name: "int".to_owned()
            })
        );
    });
}

#[rstest::rstest]
#[case::nan("float('nan')")]
#[case::infinity("float('inf')")]
#[case::negative_infinity("float('-inf')")]
fn from_python_refuses_a_non_finite_float(#[case] source: &str) {
    let host = start_host();
    host.attach(|py| {
        let object = evaluate(py, source);
        assert_eq!(from_object(object.bind(py)), Err(ValueError::NonFinite));
    });
}

#[rstest::rstest]
#[case::past_u64("2 ** 70")]
#[case::past_i64("-(2 ** 70)")]
fn from_python_refuses_an_integer_outside_i64_and_u64(#[case] source: &str) {
    let host = start_host();
    host.attach(|py| {
        let object = evaluate(py, source);
        assert_eq!(
            from_object(object.bind(py)),
            Err(ValueError::Unsupported {
                type_name: "int".to_owned()
            })
        );
    });
}

#[test]
fn from_python_reads_a_list_nested_64_levels() {
    let host = start_host();
    host.attach(|py| {
        let object = nested_list(py, MAX_DEPTH);
        assert_eq!(
            from_object(object.bind(py)).unwrap(),
            nested_array(MAX_DEPTH)
        );
    });
}

#[test]
fn from_python_refuses_a_list_nested_65_levels() {
    let host = start_host();
    host.attach(|py| {
        let object = nested_list(py, MAX_DEPTH.saturating_add(1));
        assert_eq!(from_object(object.bind(py)), Err(ValueError::TooDeep));
    });
}

#[test]
fn from_python_refuses_a_tuple_nested_65_levels() {
    let host = start_host();
    host.attach(|py| {
        let object = nested_tuple(py, MAX_DEPTH.saturating_add(1));
        assert_eq!(from_object(object.bind(py)), Err(ValueError::TooDeep));
    });
}

#[test]
fn from_python_refuses_a_dict_nested_65_levels() {
    let host = start_host();
    host.attach(|py| {
        let object = nested_dict(py, MAX_DEPTH.saturating_add(1));
        assert_eq!(from_object(object.bind(py)), Err(ValueError::TooDeep));
    });
}

#[test]
fn to_python_reads_an_array_nested_64_levels() {
    let host = start_host();
    host.attach(|py| {
        let value = nested_array(MAX_DEPTH);
        let object = to_object(py, &value).unwrap();
        assert_eq!(from_object(&object).unwrap(), value);
    });
}

#[test]
fn to_python_refuses_an_array_nested_65_levels() {
    let host = start_host();
    host.attach(|py| {
        let value = nested_array(MAX_DEPTH.saturating_add(1));
        let error = to_object(py, &value).unwrap_err();
        assert!(
            error.to_string().contains("nests deeper than 64"),
            "got: {error}"
        );
    });
}

#[test]
fn to_python_refuses_an_object_nested_65_levels() {
    let host = start_host();
    host.attach(|py| {
        let value = nested_object(MAX_DEPTH.saturating_add(1));
        let error = to_object(py, &value).unwrap_err();
        assert!(
            error.to_string().contains("nests deeper than 64"),
            "got: {error}"
        );
    });
}

fn json_values() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(Value::from),
        any::<u64>().prop_map(Value::from),
        any::<f64>()
            .prop_filter("a JSON float is finite", |value| value.is_finite())
            .prop_map(|value| Value::Number(Number::from_f64(value).unwrap())),
        any::<String>().prop_map(Value::String),
    ];
    leaf.prop_recursive(3, 24, 4, |inner| {
        prop_oneof![
            arbitrary_vec(inner.clone(), 0..4).prop_map(Value::Array),
            btree_map(any::<String>(), inner, 0..4)
                .prop_map(|entries| Value::Object(entries.into_iter().collect())),
        ]
    })
}

proptest! {
    #[test]
    fn generated_values_round_trip_through_python(value in json_values()) {
        let host = start_host();
        host.attach(|py| {
            let object = to_object(py, &value).unwrap();
            let back = from_object(&object).unwrap();
            assert_eq!(back, value);
        });
    }
}
