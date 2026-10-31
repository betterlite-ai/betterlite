CREATE TABLE "LiteLLM_ProxyModelTable" (
    model_id text PRIMARY KEY, model_name text NOT NULL, litellm_params jsonb NOT NULL,
    created_by text NOT NULL, updated_by text NOT NULL
);
CREATE TABLE "LiteLLM_CredentialsTable" (
    credential_id text PRIMARY KEY, credential_name text NOT NULL, credential_values jsonb NOT NULL,
    created_by text NOT NULL, updated_by text NOT NULL
);
CREATE TABLE "LiteLLM_Config" (param_name text PRIMARY KEY, param_value jsonb);
