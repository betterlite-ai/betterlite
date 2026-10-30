INSERT INTO "LiteLLM_ProxyModelTable" (model_id, model_name, litellm_params, created_by, updated_by)
VALUES ('one', 'test', '{"nested":["v2:gcm:model",{"name":"plain"}],"number":1}', 'test', 'test');
INSERT INTO "LiteLLM_CredentialsTable" (credential_id, credential_name, credential_values, created_by, updated_by)
VALUES ('one', 'test', '{"key":"v2:gcm:credential","legacy":"AAECAwQFBgcICQoLDA0ODxAREhMUFRYXbRIvLOA7WUwu8UmdnopPhr7H5AqjAg=="}', 'test', 'test');
CREATE TABLE "LiteLLM_MCPServerTable" (credentials jsonb, env_vars jsonb, static_headers jsonb, env jsonb);
INSERT INTO "LiteLLM_MCPServerTable" VALUES ('{"key":"v2:gcm:credentials"}', '[{"value":"v2:gcm:env_vars"}]', '{"a":"one","b":"two"}', '"{\"a\":\"three\"}"');
INSERT INTO "LiteLLM_MCPServerTable" VALUES (NULL, NULL, '"v2:gcm:invalid"', '{}');
CREATE TABLE "LiteLLM_MCPUserCredentials" (credential_b64 text);
INSERT INTO "LiteLLM_MCPUserCredentials" VALUES ('v2:gcm:user'), ('123');
CREATE TABLE "LiteLLM_MCPUserEnvVars" (values_b64 text);
INSERT INTO "LiteLLM_MCPUserEnvVars" VALUES ('v2:gcm:user_env');
INSERT INTO "LiteLLM_Config" (param_name, param_value) VALUES ('environment_variables', '{"key":"v2:gcm:config"}');
