INSERT INTO "LiteLLM_TeamTable" (team_id) VALUES ('t');
INSERT INTO "LiteLLM_TeamTable" (team_id, admins, members, models, metadata)
VALUES ('strings', '{}', '{}', '{}', '"{\"stored\": true}"'::jsonb);
INSERT INTO "LiteLLM_UserTable" (user_id) VALUES ('u');
INSERT INTO "LiteLLM_BudgetTable" (budget_id, max_budget, rpm_limit, tpm_limit, created_by, updated_by)
VALUES ('b', 1.5, 0, -2, 'test', 'test');
INSERT INTO "LiteLLM_TeamMembership" (user_id, team_id, budget_id) VALUES ('u', 't', 'b');
