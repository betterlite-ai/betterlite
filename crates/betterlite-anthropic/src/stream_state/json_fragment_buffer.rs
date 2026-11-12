pub trait AccumulatedJson {
    fn append(&mut self, fragment: &str);
    fn snapshot(&self) -> String;
    fn set(&mut self, value: String);
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JsonFragmentBuffer {
    // Fragments stay apart until a snapshot joins them, so append stays cheap.
    fragments: Vec<String>,
}

impl AccumulatedJson for JsonFragmentBuffer {
    fn append(&mut self, fragment: &str) {
        self.fragments.push(fragment.to_owned());
    }

    fn snapshot(&self) -> String {
        self.fragments.concat()
    }

    fn set(&mut self, value: String) {
        self.fragments = vec![value];
    }
}
