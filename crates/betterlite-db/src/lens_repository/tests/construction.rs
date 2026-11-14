#![cfg(test)]

use crate::daily_spend_sql::bind::SqlBind;
use crate::lens_repository::database::{Database, ExecuteFuture, QueryFuture, Row};
use crate::lens_repository::repository::LensRepository;
use crate::lens_repository::support::{LENS_ID, lens};

struct Fixed {
    data: String,
}

impl Database for Fixed {
    fn query_raw<'database>(
        &'database self,
        _query: &'database str,
        _args: &'database [SqlBind],
    ) -> QueryFuture<'database> {
        let data = self.data.clone();
        Box::pin(async move { Ok(vec![Row { data }]) })
    }

    fn execute_raw<'database>(
        &'database self,
        _query: &'database str,
        _args: &'database [SqlBind],
    ) -> ExecuteFuture<'database> {
        Box::pin(async move { Ok(1) })
    }
}

#[tokio::test]
async fn the_repository_answers_from_the_database_it_was_built_with() {
    let expected = lens(LENS_ID);
    let repository = LensRepository {
        database: Fixed {
            data: serde_json::to_string(&expected).unwrap(),
        },
    };
    assert_eq!(repository.lenses().await.unwrap(), vec![expected]);
}
