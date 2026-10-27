from typing import TypeAlias

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

NAMES = (
    ("aws_access_key_id", "AWS_ACCESS_KEY_ID"),
    ("aws_secret_access_key", "AWS_SECRET_ACCESS_KEY"),
    ("aws_session_token", "AWS_SESSION_TOKEN"),
    ("aws_region_name", "AWS_REGION_NAME"),
    ("aws_session_name", "AWS_SESSION_NAME"),
    ("aws_profile_name", "AWS_PROFILE_NAME"),
    ("aws_role_name", "AWS_ROLE_NAME"),
    ("aws_web_identity_token", "AWS_WEB_IDENTITY_TOKEN"),
    ("aws_sts_endpoint", "AWS_STS_ENDPOINT"),
    ("aws_external_id", "AWS_EXTERNAL_ID"),
    ("aws_role_arn", "AWS_ROLE_ARN"),
    ("aws_web_identity_token_file", "AWS_WEB_IDENTITY_TOKEN_FILE"),
    ("aws_bearer_token_bedrock", "AWS_BEARER_TOKEN_BEDROCK"),
)


def run(input_value: dict[str, Json]) -> Json:
    deployment = input_value.get("deployment")
    module = input_value.get("litellm_settings")
    environment = input_value.get("env")
    deployment_map = deployment if isinstance(deployment, dict) else {}
    module_map = module if isinstance(module, dict) else {}
    environment_map = environment if isinstance(environment, dict) else {}
    result: dict[str, Json] = {}
    for name, variable in NAMES:
        result[name] = deployment_map.get(name, module_map.get(name, environment_map.get(variable)))
    result["aws_session_tags"] = deployment_map.get(
        "aws_session_tags", module_map.get("aws_session_tags")
    )
    return result
