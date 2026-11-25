from typing import Literal, NotRequired, TypedDict

from litellm.proxy.management_endpoints import config_override_endpoints
from litellm.types.proxy.management_endpoints.config_overrides import (
    CyberArkConfig,
    HashicorpVaultConfig,
)
from pydantic import BaseModel, JsonValue, TypeAdapter

OBJECT_ADAPTER = TypeAdapter(dict[str, JsonValue])

HASHICORP_VAULT = "hashicorp_vault"
CYBERARK = "cyberark"

MODELS: dict[str, type[BaseModel]] = {
    HASHICORP_VAULT: HashicorpVaultConfig,
    CYBERARK: CyberArkConfig,
}

SENSITIVE_FIELDS: dict[str, set[str]] = {
    HASHICORP_VAULT: config_override_endpoints.HASHICORP_SENSITIVE_FIELDS,
    CYBERARK: config_override_endpoints.CYBERARK_SENSITIVE_FIELDS,
}

ENV_MAPPINGS: dict[str, dict[str, str]] = {
    HASHICORP_VAULT: config_override_endpoints.HASHICORP_ENV_VAR_MAPPING,
    CYBERARK: config_override_endpoints.CYBERARK_ENV_VAR_MAPPING,
}


class Input(TypedDict):
    function: Literal["mask", "audit", "schema", "project"]
    kind: str
    config: NotRequired[dict[str, JsonValue]]
    raw: NotRequired[str | dict[str, JsonValue]]


def config_of(input_value: Input) -> dict[str, JsonValue]:
    config = input_value.get("config")
    if config is None:
        raise ValueError("the mask function needs a config block")
    return config


def raw_of(input_value: Input) -> str | dict[str, JsonValue]:
    raw = input_value.get("raw")
    if raw is None:
        raise ValueError("the project function needs a raw value")
    return raw


def run(input_value: Input) -> JsonValue:
    function = input_value["function"]
    kind = input_value["kind"]
    if function == "schema":
        schema = config_override_endpoints._build_field_schema(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/config_override_endpoints.py#L219-L232
            MODELS[kind]
        )
        return OBJECT_ADAPTER.validate_python(schema)
    if function == "mask":
        masked = config_override_endpoints._mask_sensitive_fields(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/config_override_endpoints.py#L183-L191
            config_of(input_value), SENSITIVE_FIELDS[kind]
        )
        return OBJECT_ADAPTER.validate_python(masked)
    if function == "audit":
        redacted = config_override_endpoints._redact_config(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/config_override_endpoints.py#L71-L81
            input_value.get("config")
        )
        return OBJECT_ADAPTER.validate_python(redacted)
    if function == "project":
        parsed = config_override_endpoints._parse_config_value(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/config_override_endpoints.py#L235-L239
            raw_of(input_value)
        )
        projected: dict[str, JsonValue] = {}
        for field_name, env_var in ENV_MAPPINGS[kind].items():
            value = parsed.get(field_name)
            projected[env_var] = None if value is None or value == "" else str(value)
        return projected
    raise ValueError(f"unsupported override_projection function {function}")
