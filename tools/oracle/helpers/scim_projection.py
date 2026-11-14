import asyncio
import logging
import sys
from types import ModuleType
from typing import Final, Literal, TypedDict

from litellm.proxy._types import LiteLLM_TeamTable, LiteLLM_UserTable
from litellm.proxy.management_endpoints.scim.scim_transformations import (
    ScimTransformations,
)
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

JSON: Final = TypeAdapter(JsonValue)
PROXY_MODULE: Final = "litellm.proxy.proxy_server"
LOGGER_NAME: Final = "LiteLLM Proxy"


class ConnectedDatabase:
    pass


class ProxyServer(ModuleType):
    prisma_client: ConnectedDatabase | None = None


class Input(TypedDict):
    kind: Literal["user", "group"]
    value: dict[str, JsonValue]


class WarningRecorder(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.DEBUG)
        self.messages: list[JsonValue] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())


# the transform reads the module global at call time, and only tests it for None here
def connected_proxy() -> ProxyServer:
    module = ProxyServer(PROXY_MODULE)
    module.prisma_client = ConnectedDatabase()
    return module


def user_output(value: dict[str, JsonValue]) -> JsonValue:
    logger = logging.getLogger(LOGGER_NAME)
    recorder = WarningRecorder()
    previous_handlers = logger.handlers
    previous_level = logger.level
    logger.handlers = [recorder]
    logger.setLevel(logging.DEBUG)
    saved = sys.modules.get(PROXY_MODULE)
    sys.modules[PROXY_MODULE] = connected_proxy()
    try:
        # teams stay empty so the projection needs no team lookup
        user = LiteLLM_UserTable.model_validate({**value, "teams": []})
        record = asyncio.run(
            ScimTransformations.transform_litellm_user_to_scim_user(user)
        )
        user_json = JSON.validate_json(record.model_dump_json(by_alias=True))
    finally:
        if saved is None:
            sys.modules.pop(PROXY_MODULE, None)
        else:
            sys.modules[PROXY_MODULE] = saved
        logger.handlers = previous_handlers
        logger.setLevel(previous_level)
    return {"user": user_json, "warnings": recorder.messages}


def group_output(value: dict[str, JsonValue]) -> JsonValue:
    team = LiteLLM_TeamTable.model_validate(value)
    saved = sys.modules.get(PROXY_MODULE)
    sys.modules[PROXY_MODULE] = connected_proxy()
    try:
        group = asyncio.run(
            ScimTransformations.transform_litellm_team_to_scim_group(team)
        )
        return JSON.validate_json(group.model_dump_json())
    finally:
        if saved is None:
            sys.modules.pop(PROXY_MODULE, None)
        else:
            sys.modules[PROXY_MODULE] = saved


def run(input_value: Input) -> JsonValue:
    if input_value["kind"] == "user":
        return user_output(input_value["value"])
    return group_output(input_value["value"])
