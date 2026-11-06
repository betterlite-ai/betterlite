use betterlite_models::decisions::Json;
use rstest::rstest;
use std::sync::Mutex;

use crate::fields::response::{ResponsePrivacy, ServedTexts, get_final_response_obj};

use super::{decode, object};

#[derive(Default)]
struct RecordingPrivacy {
    calls: Mutex<Vec<String>>,
    served: Mutex<Option<ServedTexts>>,
}

impl ResponsePrivacy for RecordingPrivacy {
    fn overlay(&self, response: Json, served_texts: Option<&ServedTexts>) -> Json {
        self.calls.lock().unwrap().push("overlay".to_owned());
        *self.served.lock().unwrap() = served_texts.cloned();
        response
    }

    fn redact(&self, response: Json) -> Json {
        self.calls.lock().unwrap().push("redact".to_owned());
        response
    }
}

#[rstest]
#[case::response_object_wins(Some(r#"{"id":"x"}"#), r#""text""#, r#"{"id":"x"}"#)]
#[case::text_init_is_kept(None, r#""text""#, r#""text""#)]
#[case::list_init_is_kept(None, r#"["a"]"#, r#"["a"]"#)]
#[case::other_init_becomes_an_empty_object(None, "7", "{}")]
#[case::absent_init_becomes_an_empty_object(None, "null", "{}")]
fn logging_fields_behavior_tests_behavior(
    #[case] response_obj: Option<&str>,
    #[case] init: &str,
    #[case] expected: &str,
) {
    let privacy = RecordingPrivacy::default();
    let fields = response_obj.map(object);
    let init_value = decode(init);
    let served: ServedTexts = vec![Some("served".to_owned())];
    let actual = get_final_response_obj(fields.as_ref(), &init_value, Some(&served), &privacy);
    assert_eq!(actual, decode(expected));
    assert_eq!(
        *privacy.calls.lock().unwrap(),
        vec!["overlay".to_owned(), "redact".to_owned()]
    );
    assert_eq!(*privacy.served.lock().unwrap(), Some(served));
}
