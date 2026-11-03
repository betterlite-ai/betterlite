use crate::postgres_store::errors::{DbError, backend};
use betterlite_roi::records::ROISpendRecord;
use betterlite_roi::settings::normalize_email;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use std::collections::{HashMap, HashSet};
use tokio_postgres::GenericClient;

const GROUP_SQL: &str = r#"SELECT user_id, date, SUM(spend)::numeric AS spend, SUM(api_requests)::bigint AS api_requests
    FROM "LiteLLM_DailyUserSpend"
    WHERE date >= $1 AND date <= $2
    GROUP BY user_id, date
    ORDER BY date ASC"#;

const USER_EMAIL_SQL: &str =
    r#"SELECT user_id, user_email FROM "LiteLLM_UserTable" WHERE user_id = ANY($1)"#;

pub async fn read_spend<C>(
    client: &C,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Vec<ROISpendRecord>, DbError>
where
    C: GenericClient + Sync,
{
    let groups = client
        .query(GROUP_SQL, &[&start.to_string(), &end.to_string()])
        .await
        .map_err(backend)?;
    let user_ids: HashSet<String> = groups
        .iter()
        .filter_map(|row| row.get::<_, Option<String>>("user_id"))
        .collect();
    let emails = user_emails(client, user_ids).await?;
    Ok(groups
        .iter()
        .map(|row| {
            let user_id = row.get::<_, Option<String>>("user_id").unwrap_or_default();
            let email = emails
                .get(&user_id)
                .cloned()
                .unwrap_or_else(|| normalize_email(&user_id));
            ROISpendRecord {
                date: row.get("date"),
                user_id,
                email,
                spend: row.get::<_, Decimal>("spend"),
                requests: row.get::<_, i64>("api_requests"),
            }
        })
        .collect())
}

async fn user_emails<C>(
    client: &C,
    user_ids: HashSet<String>,
) -> Result<HashMap<String, String>, DbError>
where
    C: GenericClient + Sync,
{
    if user_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let ids: Vec<String> = user_ids.into_iter().collect();
    let rows = client
        .query(USER_EMAIL_SQL, &[&ids])
        .await
        .map_err(backend)?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            let user_id: String = row.get("user_id");
            let email = normalize_email(
                &row.get::<_, Option<String>>("user_email")
                    .unwrap_or_default(),
            );
            (!email.is_empty()).then_some((user_id, email))
        })
        .collect())
}

mod tests;
