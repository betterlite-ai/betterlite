CREATE TABLE IF NOT EXISTS "LiteLLM_UserTable" (user_id TEXT PRIMARY KEY, user_email TEXT);
CREATE TABLE IF NOT EXISTS "LiteLLM_TeamTable" (team_id TEXT PRIMARY KEY, team_alias TEXT);
CREATE TABLE IF NOT EXISTS "LiteLLM_VerificationToken" (token TEXT PRIMARY KEY, user_id TEXT NOT NULL REFERENCES "LiteLLM_UserTable"(user_id), team_id TEXT REFERENCES "LiteLLM_TeamTable"(team_id), user_role TEXT);
CREATE TABLE IF NOT EXISTS "LiteLLM_TeamMembership" (user_id TEXT NOT NULL REFERENCES "LiteLLM_UserTable"(user_id), team_id TEXT NOT NULL REFERENCES "LiteLLM_TeamTable"(team_id), PRIMARY KEY (user_id, team_id));
CREATE TABLE IF NOT EXISTS "LiteLLM_BudgetTable" (budget_id TEXT PRIMARY KEY, user_id TEXT NOT NULL REFERENCES "LiteLLM_UserTable"(user_id), max_budget TEXT);
CREATE TABLE IF NOT EXISTS "LiteLLM_JWTMapping" (subject TEXT PRIMARY KEY, user_id TEXT NOT NULL REFERENCES "LiteLLM_UserTable"(user_id));
