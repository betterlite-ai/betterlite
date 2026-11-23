#![cfg(test)]

use pyo3::prelude::*;

use super::{
    image_dimensions, image_high_detail_bound, image_resize, image_tile_count, image_tokens,
    image_type,
};
use crate::tokens::error::NativeTokensError;

const PNG: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAABNIAAAI3";
const GIF: &str = "data:image/gif;base64,R0lGODlhKAAeAA==";
const UNKNOWN: &str = "data:image/x;base64,bm90IGFuIGltYWdl";

#[test]
fn tokens_image_reads_the_dimensions_of_a_png_and_a_gif() {
    Python::attach(|py| {
        let png = format!(r#"{{"data":"{PNG}"}}"#);
        assert_eq!(image_dimensions(py, &png).unwrap(), (1234, 567));
        let gif = format!(r#"{{"data":"{GIF}"}}"#);
        assert_eq!(image_dimensions(py, &gif).unwrap(), (40, 30));
    });
}

#[test]
fn tokens_image_defaults_the_dimensions_of_unknown_bytes() {
    Python::attach(|py| {
        let unknown = format!(r#"{{"data":"{UNKNOWN}"}}"#);
        assert_eq!(image_dimensions(py, &unknown).unwrap(), (300, 300));
    });
}

#[test]
fn tokens_image_reports_a_bad_data_url_as_a_native_error() {
    Python::attach(|py| {
        let error = image_dimensions(py, r#"{"data":"data:image/png;base64,!!!"}"#).unwrap_err();
        assert!(error.is_instance_of::<NativeTokensError>(py));
    });
}

#[test]
fn tokens_image_resizes_the_long_side_to_the_high_res_limit() {
    Python::attach(|py| {
        let wide = image_resize(py, r#"{"width":3000,"height":1000}"#).unwrap();
        assert_eq!(wide, (2000, 666));
        let small = image_resize(py, r#"{"width":768,"height":768}"#).unwrap();
        assert_eq!(small, (768, 768));
        let flat = image_resize(py, r#"{"width":3000,"height":0}"#).unwrap_err();
        assert!(flat.is_instance_of::<NativeTokensError>(py));
    });
}

#[test]
fn tokens_image_counts_the_tiles_of_a_resized_image() {
    Python::attach(|py| {
        let tiles = image_tile_count(py, r#"{"resized_width":800,"resized_height":600}"#);
        assert_eq!(tiles.unwrap(), 4);
        let custom = image_tile_count(
            py,
            r#"{"resized_width":800,"resized_height":600,"tile_width":100,"tile_height":50}"#,
        );
        assert_eq!(custom.unwrap(), 96);
    });
}

#[test]
fn tokens_image_prices_an_image_by_its_mode() {
    Python::attach(|py| {
        let high = format!(r#"{{"data":"{PNG}","mode":"high"}}"#);
        assert_eq!(image_tokens(py, &high).unwrap(), 1445);
        let gif = format!(r#"{{"data":"{GIF}","mode":"high"}}"#);
        assert_eq!(image_tokens(py, &gif).unwrap(), 255);
        let auto = format!(r#"{{"data":"{PNG}"}}"#);
        assert_eq!(image_tokens(py, &auto).unwrap(), 85);
        let low = format!(r#"{{"data":"{PNG}","mode":"low"}}"#);
        assert_eq!(image_tokens(py, &low).unwrap(), 85);
        let base = format!(r#"{{"data":"{PNG}","mode":"high","base_tokens":100}}"#);
        assert_eq!(image_tokens(py, &base).unwrap(), 1700);
        let default =
            format!(r#"{{"data":"{PNG}","mode":"high","use_default_image_token_count":true}}"#);
        assert_eq!(image_tokens(py, &default).unwrap(), 250);
    });
}

#[test]
fn tokens_image_bounds_the_high_detail_cost() {
    Python::attach(|py| {
        assert_eq!(image_high_detail_bound(py, "{}").unwrap(), 1445);
        let custom = image_high_detail_bound(py, r#"{"base_tokens":100}"#).unwrap();
        assert_eq!(custom, 1700);
    });
}

#[test]
fn tokens_image_names_the_header_of_every_supported_format() {
    assert_eq!(
        image_type(b"\x89PNG\r\n\x1a\n\0\0\0\0\0\0\0\0\0\0"),
        Some("png")
    );
    assert_eq!(image_type(b"GIF89a\x28\x00\x1e\x00"), Some("gif"));
    assert_eq!(image_type(b"\xff\xd8\xff\x00\0\0"), Some("jpeg"));
    assert_eq!(image_type(b"\x00\x00\x00\x18ftypheic"), Some("heic"));
    assert_eq!(image_type(b"RIFF\x1e\x00\x00\x00WEBPVP8X"), Some("webp"));
    assert_eq!(image_type(b"not an image"), None);
    assert_eq!(image_type(&[]), None);
}
