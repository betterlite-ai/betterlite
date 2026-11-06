pub trait CallbackSlots {
    type Logger;

    // each slot holds the logger when that callback is one; the first such callback wins
    fn registered_callbacks(&self) -> &[Option<&Self::Logger>];

    #[must_use]
    fn get_instance(&self) -> Option<&Self::Logger> {
        self.registered_callbacks().iter().find_map(|slot| *slot)
    }
}
