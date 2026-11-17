import os
from typing import Final, TypedDict

from litellm.proxy.config_resolvers.sso import (
    SSO_DESCRIPTORS,
    SSO_FIELD_ENV_VARS,
    SSO_SECRET_FIELDS,
    resolve_sso_config,
)
from pydantic import JsonValue, TypeAdapter

CONFIG: Final = TypeAdapter(dict[str, JsonValue])
PROVENANCE: Final = TypeAdapter(dict[str, str])


class Input(TypedDict):
    salt_key: str
    db: dict[str, JsonValue] | None
    env: dict[str, str]


class DescriptorView(TypedDict):
    field_name: str
    db_key: str
    env_var: str
    is_secret: bool
    default: str | None


class Output(TypedDict):
    config: dict[str, JsonValue]
    provenance: dict[str, str]
    descriptors: list[DescriptorView]
    secret_fields: list[str]
    field_env_vars: dict[str, str]


def run(input_value: Input) -> Output:
    os.environ["LITELLM_SALT_KEY"] = input_value["salt_key"]
    resolved = resolve_sso_config(input_value["db"], input_value["env"])
    return Output(
        config=CONFIG.validate_json(resolved.config.model_dump_json()),
        provenance=PROVENANCE.validate_python(resolved.provenance),
        descriptors=[
            DescriptorView(
                field_name=descriptor.field_name,
                db_key=descriptor.db_key,
                env_var=descriptor.env_var,
                is_secret=descriptor.is_secret,
                default=descriptor.default,
            )
            for descriptor in SSO_DESCRIPTORS
        ],
        secret_fields=sorted(SSO_SECRET_FIELDS),
        field_env_vars=dict(SSO_FIELD_ENV_VARS),
    )
