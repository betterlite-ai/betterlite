use betterlite_prompts::builtin::Field;
use betterlite_prompts::builtin::Message;
use indexmap::IndexMap;

pub struct TransformInput<'request> {
    pub model: &'request str,
    pub messages: &'request [Message],
    pub optional_params: &'request IndexMap<String, Field>,
}
