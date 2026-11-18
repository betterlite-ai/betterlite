pub const BASELINE: &str = include_str!("../fixtures/baseline.sql");
// betterlite-db and betterlite-server both test read_spend against these two tables
pub const READ_SPEND_TABLES: &str = include_str!("../fixtures/read_spend.sql");
// betterlite-db and betterlite-db-admin both test the offline token import against these pinned LiteLLM tables
pub const DATABASE_REPAIR_TABLES: &str = include_str!("../fixtures/database_repair.sql");
// betterlite-db and betterlite-db-admin both test the spend backfills against these pinned LiteLLM tables
pub const SPEND_BACKFILL_TABLES: &str = include_str!("../fixtures/spend_backfills.sql");
