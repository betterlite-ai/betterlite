use betterlite_models::decisions::Json;
use indexmap::IndexMap;

const RESERVATION_KEY: &str = "user_api_key_budget_reservation";
const AUTH_KEY: &str = "user_api_key_auth";
const AUTH_RESERVATION_KEY: &str = "budget_reservation";
const CALLBACK_BOUND_KEY: &str = "callback_bound";
const BUCKETS: [&str; 2] = ["metadata", "litellm_metadata"];

mod tests;

pub trait ReservationBinder: Send + Sync {
    fn bind(&self, litellm_params: &mut IndexMap<String, Json>);
}

// LiteLLM reads the stamp before the auth record, so a direct value that is not an object does not hide the record's reservation
#[must_use]
pub fn reservation_from_metadata(
    metadata: &IndexMap<String, Json>,
) -> Option<&IndexMap<String, Json>> {
    if let Some(stamped) = object_at(metadata, RESERVATION_KEY) {
        return Some(stamped);
    }
    let auth = object_at(metadata, AUTH_KEY)?;
    object_at(auth, AUTH_RESERVATION_KEY)
}

// LiteLLM's bind_budget_reservation_to_callbacks calls this with true and unbind_budget_reservation_from_callbacks with false
pub fn stamp_callback_bound(litellm_params: &mut IndexMap<String, Json>, callback_bound: bool) {
    for bucket_name in BUCKETS {
        if let Some(Json::Object(bucket)) = litellm_params.get_mut(bucket_name)
            && let Some(reservation) = reservation_mut(bucket)
        {
            reservation.insert(CALLBACK_BOUND_KEY.to_owned(), Json::Bool(callback_bound));
        }
    }
}

fn object_at<'map>(
    map: &'map IndexMap<String, Json>,
    key: &str,
) -> Option<&'map IndexMap<String, Json>> {
    let Some(Json::Object(found)) = map.get(key) else {
        return None;
    };
    Some(found)
}

// The shared lookup borrows the bucket, so the stamp resolves the same two slots again mutably
fn reservation_mut(metadata: &mut IndexMap<String, Json>) -> Option<&mut IndexMap<String, Json>> {
    if let Some(Json::Object(stamped)) = metadata.get_mut(RESERVATION_KEY) {
        return Some(stamped);
    }
    let Some(Json::Object(auth)) = metadata.get_mut(AUTH_KEY) else {
        return None;
    };
    let Some(Json::Object(reservation)) = auth.get_mut(AUTH_RESERVATION_KEY) else {
        return None;
    };
    Some(reservation)
}
