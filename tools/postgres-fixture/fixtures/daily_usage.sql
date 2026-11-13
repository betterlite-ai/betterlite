CREATE TABLE IF NOT EXISTS "LiteLLM_DailyUserSpend" (
    "id" TEXT NOT NULL,
    "user_id" TEXT,
    "date" TEXT NOT NULL,
    "api_key" TEXT NOT NULL,
    "model" TEXT,
    "model_group" TEXT,
    "custom_llm_provider" TEXT,
    "prompt_tokens" BIGINT NOT NULL DEFAULT 0,
    "completion_tokens" BIGINT NOT NULL DEFAULT 0,
    "spend" DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    "created_at" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updated_at" TIMESTAMP(3) NOT NULL,
    "api_requests" BIGINT NOT NULL DEFAULT 0,
    "failed_requests" BIGINT NOT NULL DEFAULT 0,
    "successful_requests" BIGINT NOT NULL DEFAULT 0,
    "cache_creation_input_tokens" BIGINT NOT NULL DEFAULT 0,
    "cache_read_input_tokens" BIGINT NOT NULL DEFAULT 0,
    "mcp_namespaced_tool_name" TEXT,
    "endpoint" TEXT,
    "compression_saved_tokens" BIGINT NOT NULL DEFAULT 0,
    "compression_savings_spend" DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    "prompt_caching_savings_spend" DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    "gateway_injected_caching_savings_spend" DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    "autorouter_savings_spend" DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    "total_response_time_ms" BIGINT NOT NULL DEFAULT 0,
    "timed_requests" BIGINT NOT NULL DEFAULT 0,

    CONSTRAINT "LiteLLM_DailyUserSpend_pkey" PRIMARY KEY ("id")
);

CREATE INDEX IF NOT EXISTS "LiteLLM_DailyUserSpend_date_idx" ON "LiteLLM_DailyUserSpend"("date");

CREATE INDEX IF NOT EXISTS "LiteLLM_DailyUserSpend_user_id_date_idx" ON "LiteLLM_DailyUserSpend"("user_id", "date");

CREATE INDEX IF NOT EXISTS "LiteLLM_DailyUserSpend_api_key_idx" ON "LiteLLM_DailyUserSpend"("api_key");

CREATE INDEX IF NOT EXISTS "LiteLLM_DailyUserSpend_model_idx" ON "LiteLLM_DailyUserSpend"("model");

CREATE INDEX IF NOT EXISTS "LiteLLM_DailyUserSpend_mcp_namespaced_tool_name_idx" ON "LiteLLM_DailyUserSpend"("mcp_namespaced_tool_name");

CREATE INDEX IF NOT EXISTS "LiteLLM_DailyUserSpend_endpoint_idx" ON "LiteLLM_DailyUserSpend"("endpoint");

CREATE UNIQUE INDEX IF NOT EXISTS "LiteLLM_DailyUserSpend_user_id_date_api_key_model_custom_ll_key" ON "LiteLLM_DailyUserSpend"("user_id", "date", "api_key", "model", "custom_llm_provider", "mcp_namespaced_tool_name", "endpoint");

CREATE TABLE IF NOT EXISTS "LiteLLM_DailyTeamSpend" (
    "id" TEXT NOT NULL,
    "team_id" TEXT,
    "date" TEXT NOT NULL,
    "api_key" TEXT NOT NULL,
    "model" TEXT,
    "model_group" TEXT,
    "custom_llm_provider" TEXT,
    "prompt_tokens" BIGINT NOT NULL DEFAULT 0,
    "completion_tokens" BIGINT NOT NULL DEFAULT 0,
    "spend" DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    "api_requests" BIGINT NOT NULL DEFAULT 0,
    "successful_requests" BIGINT NOT NULL DEFAULT 0,
    "failed_requests" BIGINT NOT NULL DEFAULT 0,
    "created_at" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updated_at" TIMESTAMP(3) NOT NULL,
    "cache_creation_input_tokens" BIGINT NOT NULL DEFAULT 0,
    "cache_read_input_tokens" BIGINT NOT NULL DEFAULT 0,
    "mcp_namespaced_tool_name" TEXT,
    "endpoint" TEXT,
    "ptu_flat_cost" DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    "compression_saved_tokens" BIGINT NOT NULL DEFAULT 0,
    "compression_savings_spend" DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    "prompt_caching_savings_spend" DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    "autorouter_savings_spend" DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    "gateway_injected_caching_savings_spend" DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    "total_response_time_ms" BIGINT NOT NULL DEFAULT 0,
    "timed_requests" BIGINT NOT NULL DEFAULT 0,

    CONSTRAINT "LiteLLM_DailyTeamSpend_pkey" PRIMARY KEY ("id")
);

CREATE INDEX IF NOT EXISTS "LiteLLM_DailyTeamSpend_date_idx" ON "LiteLLM_DailyTeamSpend"("date");

CREATE INDEX IF NOT EXISTS "LiteLLM_DailyTeamSpend_team_id_date_idx" ON "LiteLLM_DailyTeamSpend"("team_id", "date");

CREATE INDEX IF NOT EXISTS "LiteLLM_DailyTeamSpend_api_key_idx" ON "LiteLLM_DailyTeamSpend"("api_key");

CREATE INDEX IF NOT EXISTS "LiteLLM_DailyTeamSpend_model_idx" ON "LiteLLM_DailyTeamSpend"("model");

CREATE INDEX IF NOT EXISTS "LiteLLM_DailyTeamSpend_mcp_namespaced_tool_name_idx" ON "LiteLLM_DailyTeamSpend"("mcp_namespaced_tool_name");

CREATE INDEX IF NOT EXISTS "LiteLLM_DailyTeamSpend_endpoint_idx" ON "LiteLLM_DailyTeamSpend"("endpoint");

CREATE UNIQUE INDEX IF NOT EXISTS "LiteLLM_DailyTeamSpend_team_id_date_api_key_model_custom_ll_key" ON "LiteLLM_DailyTeamSpend"("team_id", "date", "api_key", "model", "custom_llm_provider", "mcp_namespaced_tool_name", "endpoint");

CREATE TABLE IF NOT EXISTS "LiteLLM_DailyTagSpend" (
    "id" TEXT NOT NULL,
    "tag" TEXT,
    "date" TEXT NOT NULL,
    "api_key" TEXT NOT NULL,
    "model" TEXT,
    "model_group" TEXT,
    "custom_llm_provider" TEXT,
    "prompt_tokens" BIGINT NOT NULL DEFAULT 0,
    "completion_tokens" BIGINT NOT NULL DEFAULT 0,
    "cache_read_input_tokens" BIGINT NOT NULL DEFAULT 0,
    "cache_creation_input_tokens" BIGINT NOT NULL DEFAULT 0,
    "spend" DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    "api_requests" BIGINT NOT NULL DEFAULT 0,
    "successful_requests" BIGINT NOT NULL DEFAULT 0,
    "failed_requests" BIGINT NOT NULL DEFAULT 0,
    "created_at" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updated_at" TIMESTAMP(3) NOT NULL,
    "mcp_namespaced_tool_name" TEXT,
    "request_id" TEXT,
    "endpoint" TEXT,
    "compression_saved_tokens" BIGINT NOT NULL DEFAULT 0,
    "compression_savings_spend" DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    "prompt_caching_savings_spend" DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    "autorouter_savings_spend" DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    "gateway_injected_caching_savings_spend" DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    "total_response_time_ms" BIGINT NOT NULL DEFAULT 0,
    "timed_requests" BIGINT NOT NULL DEFAULT 0,

    CONSTRAINT "LiteLLM_DailyTagSpend_pkey" PRIMARY KEY ("id")
);

CREATE INDEX IF NOT EXISTS "LiteLLM_DailyTagSpend_date_idx" ON "LiteLLM_DailyTagSpend"("date");

CREATE INDEX IF NOT EXISTS "LiteLLM_DailyTagSpend_tag_date_idx" ON "LiteLLM_DailyTagSpend"("tag", "date");

CREATE INDEX IF NOT EXISTS "LiteLLM_DailyTagSpend_api_key_idx" ON "LiteLLM_DailyTagSpend"("api_key");

CREATE INDEX IF NOT EXISTS "LiteLLM_DailyTagSpend_model_idx" ON "LiteLLM_DailyTagSpend"("model");

CREATE INDEX IF NOT EXISTS "LiteLLM_DailyTagSpend_mcp_namespaced_tool_name_idx" ON "LiteLLM_DailyTagSpend"("mcp_namespaced_tool_name");

CREATE INDEX IF NOT EXISTS "LiteLLM_DailyTagSpend_endpoint_idx" ON "LiteLLM_DailyTagSpend"("endpoint");

CREATE UNIQUE INDEX IF NOT EXISTS "LiteLLM_DailyTagSpend_tag_date_api_key_model_custom_llm_pro_key" ON "LiteLLM_DailyTagSpend"("tag", "date", "api_key", "model", "custom_llm_provider", "mcp_namespaced_tool_name", "endpoint");
