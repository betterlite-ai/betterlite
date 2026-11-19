use betterlite_openai::api::image_contracts::records::{ImageHeaders, ImageParams};

// What these two methods hand logging_obj.pre_call: the prompt, the empty key they log, and the request they built
pub struct PreCallRecord<'record> {
    pub input: &'record str,
    pub api_key: &'record str,
    pub api_base: &'record str,
    pub headers: &'record ImageHeaders,
    pub complete_input_dict: &'record ImageParams,
}

pub trait ImageGenerationLogger {
    fn pre_call(&mut self, record: PreCallRecord<'_>);
}
