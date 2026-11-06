use std::convert::Infallible;
use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use betterlite_server::password_policy::hashing::{PasswordHasher, SaltSource};
use betterlite_server::password_policy::migration::{
    PasswordStore, StoredPassword, migrate_passwords_to_scrypt,
};
use secrecy::SecretString;
use serde_json::{Value, json};

use super::{User, failure};
use crate::helpers::HelperResult;

struct Rows(Mutex<Vec<(String, String)>>);

impl PasswordStore for Rows {
    type Error = Infallible;

    fn users_with_passwords(
        &self,
    ) -> impl Future<Output = Result<Vec<StoredPassword>, Infallible>> + Send {
        let rows = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        ready(Ok(rows
            .iter()
            .map(|(user_id, password)| StoredPassword {
                user_id: user_id.clone(),
                password: SecretString::from(password.clone()),
            })
            .collect()))
    }

    fn set_password(
        &self,
        user_id: &str,
        stored_hash: &str,
    ) -> impl Future<Output = Result<(), Infallible>> + Send {
        let mut rows = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        for row in rows.iter_mut().filter(|row| row.0 == user_id) {
            stored_hash.clone_into(&mut row.1);
        }
        drop(rows);
        ready(Ok(()))
    }
}

pub(super) async fn migrate<S>(users: &[User], hasher: &PasswordHasher<S>) -> HelperResult
where
    S: SaltSource,
{
    let store = Rows(Mutex::new(
        users
            .iter()
            .map(|user| (user.user_id.clone(), user.password.clone()))
            .collect(),
    ));
    let message = migrate_passwords_to_scrypt(&store, hasher)
        .await
        .map_err(|error| failure(&error))?;
    let rows: Vec<Value> = store
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .map(|(user_id, password)| json!({ "user_id": user_id, "password": password }))
        .collect();
    Ok(json!({ "message": message, "rows": rows }))
}
