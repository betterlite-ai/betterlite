use super::render::RenderError;

const TEMPLATE_BYTES: usize = 0x0001_0000;
const OUTPUT_BYTES: usize = 0x0010_0000;

pub(super) const fn check_template(content: &str) -> Result<(), RenderError> {
    if content.len() > TEMPLATE_BYTES {
        Err(RenderError::TemplateTooLarge)
    } else {
        Ok(())
    }
}

pub(super) fn check_output(rendered: String) -> Result<String, RenderError> {
    if rendered.len() > OUTPUT_BYTES {
        Err(RenderError::OutputTooLarge)
    } else {
        Ok(rendered)
    }
}

mod tests;
