use std::collections::{BTreeMap, BTreeSet};

use http::Method;
use serde::de::DeserializeOwned;
use url::Url;

use super::error::DebugError;
use super::wire::{RequestResponsePayload, SessionLogsPage, SpendLogRow};
use crate::commands::{Client, Payload, Request};

const SESSION_LOGS_PATH: &str = "/spend/logs/session/ui";
const PAYLOAD_PATH: &str = "/spend/logs/ui";
const SESSION_PAGE_SIZE: u32 = 100;
const TRANSPORT_BODY_CHARS: usize = 500;

pub struct SpendLogsFetcher<'client> {
    pub client: &'client Client,
}

impl SpendLogsFetcher<'_> {
    pub fn session_rows(&self, session_id: &str) -> Result<Vec<SpendLogRow>, DebugError> {
        let first = self.page(session_id, 1)?;
        let mut rows = first.data;
        for page in 2..=first.total_pages {
            rows.extend(self.page(session_id, page)?.data);
        }
        rows.sort_by_key(|row| row.start_time.clone());
        Ok(rows)
    }

    pub fn payload(&self, request_id: &str) -> Result<Option<RequestResponsePayload>, DebugError> {
        let uri = format!("{PAYLOAD_PATH}/{request_id}");
        let mut url = self.resolve(PAYLOAD_PATH)?;
        let target = url.to_string();
        url.path_segments_mut()
            .map_err(|()| DebugError::BaseUrlHoldsNoPath {
                uri: uri.clone(),
                url: target,
            })?
            .push(request_id);
        self.get(&uri, url)
    }

    pub fn payloads(
        &self,
        rows: &[SpendLogRow],
        recent_bodies: usize,
    ) -> Result<BTreeMap<String, Option<RequestResponsePayload>>, DebugError> {
        let mut wanted: BTreeSet<&str> = rows
            .iter()
            .filter(|row| row.failed())
            .map(|row| row.request_id.as_str())
            .collect();
        wanted.extend(
            rows.iter()
                .skip(rows.len().saturating_sub(recent_bodies))
                .map(|row| row.request_id.as_str()),
        );
        let mut payloads = BTreeMap::new();
        for request_id in wanted {
            payloads.insert(request_id.to_owned(), self.payload(request_id)?);
        }
        Ok(payloads)
    }

    fn page(&self, session_id: &str, page: i64) -> Result<SessionLogsPage, DebugError> {
        let mut url = self.resolve(SESSION_LOGS_PATH)?;
        url.query_pairs_mut()
            .append_pair("session_id", session_id)
            .append_pair("page", &page.to_string())
            .append_pair("page_size", &SESSION_PAGE_SIZE.to_string());
        self.get(SESSION_LOGS_PATH, url)
    }

    fn resolve(&self, uri: &str) -> Result<Url, DebugError> {
        self.client
            .resolve(uri)
            .map_err(|source| DebugError::Request {
                uri: uri.to_owned(),
                source,
            })
    }

    fn get<T: DeserializeOwned>(&self, uri: &str, url: Url) -> Result<T, DebugError> {
        let request = Request {
            method: Method::GET,
            url,
            headers: Vec::new(),
            payload: Payload::Empty,
        };
        let response = self
            .client
            .send(request)
            .map_err(|source| DebugError::Request {
                uri: uri.to_owned(),
                source,
            })?;
        if response.status >= 400 {
            return Err(DebugError::Status {
                uri: uri.to_owned(),
                status: response.status,
                body: response
                    .body
                    .utf8_chunks()
                    .flat_map(|chunk| chunk.valid().chars())
                    .take(TRANSPORT_BODY_CHARS)
                    .collect(),
            });
        }
        serde_json::from_slice(&response.body).map_err(|source| DebugError::UnexpectedShape {
            uri: uri.to_owned(),
            source,
        })
    }
}

mod tests;
