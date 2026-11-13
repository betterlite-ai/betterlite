use rstest::rstest;

use crate::ptu_pricing::ptu_identity_error;

const MISSING_WITHOUT_HINT: &str = "model_info.id is required when PTU fields are set. Without one \
     the deployment is identified by a hash of its litellm_params, so rotating a credential bills \
     the reservation a second time under the new identity. Set it to the id this deployment already \
     uses, shown by GET /model/info, so the flat cost already written stays under one identity; any \
     other value starts a second one";

const DUPLICATE: &str = "model_info.id 'dep-1' is declared on more than one deployment. Each would \
     key the same flat-cost row, so one reservation would go unbilled";

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
fn identity_error_names_the_current_id(#[case] declared_id: Option<&str>) {
    assert_eq!(
        ptu_identity_error(declared_id, false, Some("dep-1"), None),
        Some(
            "model_info.id is required when PTU fields are set. Without one the deployment is \
             identified by a hash of its litellm_params, so rotating a credential bills the \
             reservation a second time under the new identity. Set it to the id this deployment \
             already uses, dep-1, so the flat cost already written stays under one identity; any \
             other value starts a second one"
                .to_owned()
        )
    );
}

#[test]
fn identity_error_points_at_model_info_without_a_current_id() {
    assert_eq!(
        ptu_identity_error(None, false, None, None),
        Some(MISSING_WITHOUT_HINT.to_owned())
    );
    assert_eq!(
        ptu_identity_error(None, false, Some(""), None),
        Some(MISSING_WITHOUT_HINT.to_owned())
    );
}

#[test]
fn identity_error_rejects_a_duplicate_declared_id() {
    assert_eq!(
        ptu_identity_error(Some("dep-1"), true, None, None),
        Some(DUPLICATE.to_owned())
    );
}

#[test]
fn identity_error_accepts_a_unique_declared_id() {
    assert_eq!(ptu_identity_error(Some("dep-1"), false, None, None), None);
}

#[test]
fn identity_error_prefixes_the_model_name() {
    assert_eq!(
        ptu_identity_error(Some("dep-1"), true, None, Some("gpt-4o")),
        Some(format!(
            "PTU configuration on model 'gpt-4o' is invalid: {DUPLICATE}"
        ))
    );
}
