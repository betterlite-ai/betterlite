use http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use url::Url;

use crate::http_analytics_loggers::delivery::{
    DELIVERY_DEADLINE, Delivery, DeliveryError, PostRequest,
};

const JSON_CONTENT_TYPE: &str = "application/json";
const RETURN_REPRESENTATION: &str = "return=representation";
const MERGE_DUPLICATES: &str = "resolution=merge-duplicates,return=representation";

pub struct SupabaseClient<D: Delivery> {
    pub base_url: Url,
    pub key: SecretString,
    pub delivery: D,
}

impl<D: Delivery> SupabaseClient<D> {
    // postgrest-py reaches the table route with this POST for both insert() and upsert()
    pub async fn write<Body>(
        &self,
        table: &str,
        body: &Body,
        on_conflict: Option<&str>,
    ) -> Result<(), DeliveryError<D::Error>>
    where
        Body: Serialize + Sync,
    {
        let encoded = serde_json::to_vec(body).map_err(DeliveryError::Encode)?;
        let sent = self
            .delivery
            .post(self.request(table, encoded, on_conflict)?)
            .await
            .map_err(DeliveryError::Transport)?;
        if !(200..300).contains(&sent.status) {
            return Err(DeliveryError::Status {
                status: sent.status,
            });
        }
        Ok(())
    }

    fn request(
        &self,
        table: &str,
        body: Vec<u8>,
        on_conflict: Option<&str>,
    ) -> Result<PostRequest, DeliveryError<D::Error>> {
        let prefer = if on_conflict.is_some() {
            MERGE_DUPLICATES
        } else {
            RETURN_REPRESENTATION
        };
        Ok(PostRequest {
            url: self.table_url(table, on_conflict),
            headers: self.headers(prefer)?,
            body,
            deadline: DELIVERY_DEADLINE,
        })
    }

    fn table_url(&self, table: &str, on_conflict: Option<&str>) -> Url {
        let mut url = self.base_url.clone();
        let base = self.base_url.path().trim_end_matches('/');
        url.set_path(&format!("{base}/rest/v1/{table}"));
        if let Some(column) = on_conflict {
            url.query_pairs_mut().append_pair("on_conflict", column);
        }
        url
    }

    fn headers(&self, prefer: &'static str) -> Result<HeaderMap, DeliveryError<D::Error>> {
        let key = self.key.expose_secret();
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("apikey"),
            HeaderValue::try_from(key).map_err(|source| DeliveryError::Header {
                name: "supabase apikey",
                source,
            })?,
        );
        headers.insert(
            AUTHORIZATION,
            HeaderValue::try_from(format!("Bearer {key}")).map_err(|source| {
                DeliveryError::Header {
                    name: "supabase authorization",
                    source,
                }
            })?,
        );
        headers.insert(ACCEPT, HeaderValue::from_static(JSON_CONTENT_TYPE));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static(JSON_CONTENT_TYPE));
        headers.insert(
            HeaderName::from_static("prefer"),
            HeaderValue::from_static(prefer),
        );
        Ok(headers)
    }
}
