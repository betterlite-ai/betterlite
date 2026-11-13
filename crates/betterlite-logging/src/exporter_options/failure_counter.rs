pub trait CallbackFailureCounter {
    // ast-grep-ignore: no-long-names
    fn increment_callback_logging_failure(&self, callback_name: &str);
}

pub fn handle_callback_failure(callback_name: &str, counters: &[&dyn CallbackFailureCounter]) {
    let Some(counter) = counters.first() else {
        tracing::debug!(
            callback_name,
            "no registered callback counts logging failures"
        );
        return;
    };
    counter.increment_callback_logging_failure(callback_name);
}
