The pinned PostgreSQL baseline is kept with fixture assets so its source-sized SQL
does not count against the fixture implementation budget. It is loaded by the
fixture crate through include_str! and is never modified during a test run.
