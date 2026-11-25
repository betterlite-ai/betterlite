import os
from dataclasses import dataclass
from datetime import datetime
from typing import Final, Literal, TypeAlias, TypedDict

from litellm.proxy import proxy_server
from litellm.proxy.proxy_server import ProxyConfig
from pydantic import JsonValue, TypeAdapter

SALT_ENV: Final = "LITELLM_SALT_KEY"
OBJECT: Final = TypeAdapter(dict[str, JsonValue])
DATETIME: Final = TypeAdapter(datetime | None)
# LiteLLM copies the row's audit columns only for premium_user, which betterlite always is
proxy_server.premium_user = True


class ModelRowInput(TypedDict):
    model_id: str
    model_name: str
    litellm_params: dict[str, JsonValue]
    model_info: dict[str, JsonValue]
    blocked: bool
    created_at: str | None
    updated_at: str | None
    created_by: str | None
    updated_by: str | None


class ModelInfoCall(TypedDict):
    operation: Literal["model_info"]
    rows: list[ModelRowInput]


class ExportEnvCall(TypedDict):
    operation: Literal["export_env"]
    salt_key: str
    values: dict[str, JsonValue]
    keep_original: bool
    environment: dict[str, str]


Input: TypeAlias = ModelInfoCall | ExportEnvCall


@dataclass
class ModelRow:
    model_id: str
    model_name: str
    litellm_params: dict[str, JsonValue]
    model_info: dict[str, JsonValue]
    blocked: bool
    created_at: datetime | None
    updated_at: datetime | None
    created_by: str | None
    updated_by: str | None


def model_row(row: ModelRowInput) -> ModelRow:
    return ModelRow(
        model_id=row["model_id"],
        model_name=row["model_name"],
        litellm_params=row["litellm_params"],
        model_info=row["model_info"],
        blocked=row["blocked"],
        created_at=DATETIME.validate_python(row["created_at"]),
        updated_at=DATETIME.validate_python(row["updated_at"]),
        created_by=row["created_by"],
        updated_by=row["updated_by"],
    )


def model_info_answer(proxy_config: ProxyConfig, rows: list[ModelRowInput]) -> JsonValue:
    infos: list[JsonValue] = []
    for row in rows:
        info = proxy_config.get_model_info_with_id(model=model_row(row), db_model=False)  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L7241-L7277
        infos.append(OBJECT.validate_json(info.model_dump_json(exclude_none=True)))
    return infos


def export_answer(proxy_config: ProxyConfig, call: ExportEnvCall) -> JsonValue:
    if os.environ.get(SALT_ENV) != call["salt_key"]:
        raise ValueError("the case env and the helper input disagree on LITELLM_SALT_KEY")
    os.environ.update(call["environment"])
    exported = OBJECT.validate_python(
        proxy_config._decrypt_and_set_db_env_variables(call["values"], return_original_value=call["keep_original"])  # type: ignore[misc]  # noqa: SLF001, E501  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L7585-L7612
    )
    environment: dict[str, JsonValue] = {key: os.environ.get(key) for key in call["values"]}
    return {"exported": exported, "environment": environment}


def run(input_value: Input) -> JsonValue:
    proxy_config = ProxyConfig()
    match input_value["operation"]:
        case "model_info":
            return model_info_answer(proxy_config, input_value["rows"])
        case "export_env":
            return export_answer(proxy_config, input_value)
