use betterlite_a2a::card_resolver::card_url;
use betterlite_a2a::card_resolver::interfaces::normalize_interfaces;
use betterlite_a2a::card_resolver::localhost;
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::future_into_py;

use super::card;
use super::client;
use super::convert::{Call, Settings, parse};
use super::send;
use super::stream;

#[pyfunction]
pub(crate) fn create_client<'py>(py: Python<'py>, settings: &str) -> PyResult<Bound<'py, PyAny>> {
    let wire: Settings = parse(settings)?;
    future_into_py(py, async move {
        let created = client::create(wire).await?;
        Python::attach(|attached| Py::new(attached, created))
    })
}

#[pyfunction]
pub(crate) fn get_agent_card<'py>(py: Python<'py>, settings: &str) -> PyResult<Bound<'py, PyAny>> {
    let wire: Settings = parse(settings)?;
    let fetching = client::fetch_card(wire);
    future_into_py(py, fetching)
}

#[pyfunction]
pub(crate) fn send_message_direct<'py>(py: Python<'py>, call: &str) -> PyResult<Bound<'py, PyAny>> {
    let wire: Call = parse(call)?;
    let sending = send::send(None, wire);
    future_into_py(py, sending)
}

#[pyfunction]
pub(crate) fn send_message_streaming_direct<'py>(
    py: Python<'py>,
    call: &str,
) -> PyResult<Bound<'py, PyAny>> {
    let wire: Call = parse(call)?;
    let starting = async move {
        let started = stream::start(None, wire)?;
        Python::attach(|attached| Py::new(attached, started))
    };
    future_into_py(py, starting)
}

#[pyfunction]
pub(crate) fn is_localhost_or_internal_url(url: Option<&str>) -> bool {
    url.is_some_and(|candidate| localhost::is_localhost_or_internal_url(Some(candidate)))
}

#[pyfunction(name = "normalize_agent_card_interfaces")]
pub(crate) fn normalized_card(card: &str) -> PyResult<String> {
    let found = card::parse(card)?;
    let normalized = normalize_interfaces(found);
    Ok(card::text(&normalized)?)
}

#[pyfunction]
pub(crate) fn get_agent_card_url(card: &str) -> PyResult<Option<String>> {
    let found = card::parse(card)?;
    let url = card_url::get_agent_card_url(&found);
    Ok(url.map(str::to_owned))
}

#[pyfunction]
pub(crate) fn set_agent_card_url(card: &str, url: &str) -> PyResult<String> {
    let mut found = card::parse(card)?;
    card_url::set_agent_card_url(&mut found, url);
    Ok(card::text(&found)?)
}

#[pyfunction]
pub(crate) fn fix_agent_card_url(card: &str, base_url: &str) -> PyResult<String> {
    let mut found = card::parse(card)?;
    card::fix(&mut found, base_url);
    Ok(card::text(&found)?)
}

mod tests;
