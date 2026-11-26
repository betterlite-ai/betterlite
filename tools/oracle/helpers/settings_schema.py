import asyncio
from typing import TypedDict, cast

import pydantic
from litellm.proxy._types import DefaultInternalUserParams
from litellm.proxy.ui_crud_endpoints.proxy_setting_endpoints import _get_settings_with_schema  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/ui_crud_endpoints/proxy_setting_endpoints.py#L800-L849
from pydantic import JsonValue


class Input(TypedDict):
    settings_key: str
    config: dict[str, JsonValue]
    schema: dict[str, JsonValue]
    defaults: dict[str, JsonValue]


class SchemaAdapter:
    def __init__(self, schema: dict[str, JsonValue]) -> None:
        self.schema = schema

    def json_schema(self, *, by_alias: bool) -> dict[str, JsonValue]:
        if not by_alias:
            raise ValueError("by_alias must be True")
        return self.schema


class SchemaFactory:
    def __init__(self, schema: dict[str, JsonValue]) -> None:
        self.schema = schema

    def __call__(self, _settings_class: type[pydantic.BaseModel]) -> SchemaAdapter:
        return SchemaAdapter(self.schema)


def run(input_value: Input) -> dict[str, JsonValue]:
    pydantic.TypeAdapter = SchemaFactory(input_value["schema"])  # type: ignore[misc, assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/ui_crud_endpoints/proxy_setting_endpoints.py#L800-L803
    arguments = (input_value["settings_key"], DefaultInternalUserParams, input_value["config"])
    raw = asyncio.run(_get_settings_with_schema(*arguments))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/ui_crud_endpoints/proxy_setting_endpoints.py#L800-L849
    return cast(dict[str, JsonValue], raw)  # pyright: ignore[reportUnknownArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/ui_crud_endpoints/proxy_setting_endpoints.py#L800-L849
