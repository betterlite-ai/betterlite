use betterlite_logging::reservation_binding::{ReservationBinder, stamp_callback_bound};
use betterlite_models::decisions::Json;
use betterlite_redis::request_batch::lock;
use indexmap::IndexMap;

use crate::reservation_settlement::handle::ReservationHandle;

mod tests;

// the metadata holds a JSON copy of the reservation, while the request-end release reads callback_bound on the shared handle
pub struct RequestReservationBinder {
    pub reservation: Option<ReservationHandle>,
}

impl ReservationBinder for RequestReservationBinder {
    fn bind(&self, litellm_params: &mut IndexMap<String, Json>) {
        stamp_callback_bound(litellm_params, true);
        if let Some(handle) = &self.reservation {
            lock(&handle.0).callback_bound = true;
        }
    }
}
