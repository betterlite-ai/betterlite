CREATE TABLE "LiteLLM_Config" (
    param_name text PRIMARY KEY, param_value jsonb, last_run_at timestamp(3)
);
