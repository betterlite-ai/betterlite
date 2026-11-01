use std::collections::BTreeSet;

use super::identity::MemberIdentity;

#[must_use]
pub fn duplicate_member_indexes(requests: &[MemberIdentity]) -> BTreeSet<usize> {
    let mut duplicates = BTreeSet::new();
    for (index, request) in requests.iter().enumerate() {
        let earlier = requests.get(..index).unwrap_or(&[]);
        if earlier.iter().any(|other| shares_identity(request, other)) {
            duplicates.insert(index);
        }
    }
    duplicates
}

fn shares_identity(request: &MemberIdentity, other: &MemberIdentity) -> bool {
    let same_id = request.user_id.is_some() && request.user_id == other.user_id;
    let same_email = request.user_email.is_some() && request.user_email == other.user_email;
    same_id || same_email
}
