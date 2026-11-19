pub mod batch_enqueued;
pub mod failure_event;
pub mod failure_hook;
pub mod header_merge;
pub mod logging_hook;
pub mod settlement;
pub mod success_event;
pub mod success_hook;

mod apply;
mod disconnect;
mod refund_operations;
mod snapshot;

mod tests;

#[cfg(test)]
mod batch_id {
    mod tests;
}

#[cfg(test)]
mod batch_refund {
    mod tests;
}

#[cfg(test)]
mod batch_save {
    mod tests;
}

#[cfg(test)]
mod disconnect_slot {
    mod tests;
}

#[cfg(test)]
mod failure_event_application {
    mod tests;
}

#[cfg(test)]
mod failure_event_error {
    mod tests;
}

#[cfg(test)]
mod failure_event_flag {
    mod tests;
}

#[cfg(test)]
mod failure_event_idempotency {
    mod tests;
}

#[cfg(test)]
mod failure_event_partial {
    mod tests;
}

#[cfg(test)]
mod failure_event_scope {
    mod tests;
}

#[cfg(test)]
mod failure_event_slot {
    mod tests;
}

#[cfg(test)]
mod failure_hook_application {
    mod tests;
}

#[cfg(test)]
mod failure_hook_batch {
    mod tests;
}

#[cfg(test)]
mod failure_hook_error {
    mod tests;
}

#[cfg(test)]
mod failure_hook_flag {
    mod tests;
}

#[cfg(test)]
mod failure_hook_io {
    mod tests;
}

#[cfg(test)]
mod failure_hook_tpm {
    mod tests;
}

#[cfg(test)]
mod success_event_error {
    mod tests;
}

#[cfg(test)]
mod success_event_io {
    mod tests;
}

#[cfg(test)]
mod success_event_slot {
    mod tests;
}

#[cfg(test)]
mod success_event_tpm {
    mod tests;
}

#[cfg(test)]
mod success_hook_exception {
    mod tests;
}

#[cfg(test)]
mod success_hook_headers {
    mod tests;
}

#[cfg(test)]
mod success_hook_slot {
    mod tests;
}
