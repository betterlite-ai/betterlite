mod codec;
mod convert;
mod count;
mod error;
mod image;
mod runtime;
mod selection;
mod store;

use pyo3::prelude::*;

use codec::{decode_tokens, encode_text, free_tokenizer, load_tokenizer};
use count::{modified_max_tokens, token_count};
use error::NativeTokensError;
use image::{
    image_dimensions, image_high_detail_bound, image_resize, image_tile_count, image_tokens,
    image_type,
};
use selection::{legacy_message_accounting, openai_encoding_name};

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add(
        "NativeTokensError",
        module.py().get_type::<NativeTokensError>(),
    )?;
    module.add_function(wrap_pyfunction!(token_count, module)?)?;
    module.add_function(wrap_pyfunction!(encode_text, module)?)?;
    module.add_function(wrap_pyfunction!(decode_tokens, module)?)?;
    module.add_function(wrap_pyfunction!(load_tokenizer, module)?)?;
    module.add_function(wrap_pyfunction!(free_tokenizer, module)?)?;
    module.add_function(wrap_pyfunction!(image_tokens, module)?)?;
    module.add_function(wrap_pyfunction!(image_tile_count, module)?)?;
    module.add_function(wrap_pyfunction!(image_type, module)?)?;
    module.add_function(wrap_pyfunction!(image_dimensions, module)?)?;
    module.add_function(wrap_pyfunction!(image_resize, module)?)?;
    module.add_function(wrap_pyfunction!(image_high_detail_bound, module)?)?;
    module.add_function(wrap_pyfunction!(modified_max_tokens, module)?)?;
    module.add_function(wrap_pyfunction!(openai_encoding_name, module)?)?;
    module.add_function(wrap_pyfunction!(legacy_message_accounting, module)?)?;
    Ok(())
}

mod tests;
