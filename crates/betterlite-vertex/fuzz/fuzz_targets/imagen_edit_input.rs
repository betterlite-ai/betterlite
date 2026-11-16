#![no_main]

use betterlite_config::sections::vertex_schema_shaping::Settings;
use betterlite_vertex::vertex_imagen_edit::decode::parse_imagen_edit_input;
use betterlite_vertex::vertex_imagen_edit::input::read_image_bytes;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(projected) = parse_imagen_edit_input(bytes) else {
        return;
    };
    let Some(mut image) = projected else {
        return;
    };
    drop(read_image_bytes(
        &mut image,
        0,
        Settings::default().max_recurse_depth,
    ));
});
