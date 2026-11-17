use uuid::Uuid;

use crate::gemini_live_content::ids::ContentIds;

pub struct DrawnIds<'ids>(pub &'ids mut dyn FnMut() -> Uuid);

impl ContentIds for DrawnIds<'_> {
    fn next_id(&mut self) -> String {
        (self.0)().to_string()
    }
}
