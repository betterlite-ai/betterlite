CREATE TABLE "LiteLLM_TeamTable" (
    team_id TEXT NOT NULL,
    team_alias TEXT,
    organization_id TEXT,
    admins TEXT[] NOT NULL DEFAULT '{}',
    members TEXT[] NOT NULL DEFAULT '{}',
    models TEXT[] NOT NULL DEFAULT '{}',
    members_with_roles JSONB NOT NULL DEFAULT '{}',
    metadata JSONB NOT NULL DEFAULT '{}',
    model_spend JSONB NOT NULL DEFAULT '{}',
    model_max_budget JSONB NOT NULL DEFAULT '{}',
    router_settings JSONB DEFAULT '{}',
    budget_limits JSONB,
    max_budget DOUBLE PRECISION,
    spend DOUBLE PRECISION NOT NULL DEFAULT 0.0,

    CONSTRAINT "LiteLLM_TeamTable_pkey" PRIMARY KEY (team_id)
);

INSERT INTO "LiteLLM_TeamTable" (team_id, team_alias, organization_id, members_with_roles, metadata)
VALUES
    ('t', 'alpha', 'org-a', '[{"user_id":"u1","role":"admin"}]'::JSONB, '{}'::JSONB),
    ('empty', 'empty-team', 'org-a', '{}'::JSONB, '{}'::JSONB),
    ('strings', 'strings-team', 'org-a', '{}'::JSONB, '"{\"stored\": true}"'::JSONB),
    ('other', 'beta', 'org-b', '{}'::JSONB, '{}'::JSONB);
