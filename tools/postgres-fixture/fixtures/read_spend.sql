CREATE TABLE "LiteLLM_DailyUserSpend" (
    user_id text, date text NOT NULL,
    spend double precision NOT NULL DEFAULT 0.0, api_requests bigint NOT NULL DEFAULT 0
);
CREATE TABLE "LiteLLM_UserTable" (user_id text PRIMARY KEY, user_email text);
