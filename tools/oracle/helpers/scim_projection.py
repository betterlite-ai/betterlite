import asyncio
import logging
from typing import Literal, TypedDict
from unittest.mock import patch

from litellm.proxy import proxy_server
from litellm.proxy._types import LiteLLM_TeamTable, LiteLLM_UserTable
from litellm.proxy.management_endpoints.scim.scim_transformations import (
    ScimTransformations,
)
from typing_extensions import override


class Input(TypedDict):
    kind: Literal["user", "group"]
    value: dict[str, object]


class WarningRecorder(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.DEBUG)
        self.messages: list[str] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())


def user_output(value: dict[str, object]) -> dict[str, object]:
    logger = logging.getLogger("LiteLLM Proxy")
    recorder = WarningRecorder()
    previous_handlers = logger.handlers
    previous_level = logger.level
    logger.handlers = [recorder]
    logger.setLevel(logging.DEBUG)
    try:
        # teams stay empty so the projection needs no team lookup
        user = LiteLLM_UserTable.model_validate({**value, "teams": []})
        with patch.object(proxy_server, "prisma_client", object()):
            record = asyncio.run(
                ScimTransformations.transform_litellm_user_to_scim_user(user)
            )
    finally:
        logger.handlers = previous_handlers
        logger.setLevel(previous_level)
    return {"user": record.model_dump(by_alias=True), "warnings": recorder.messages}


def run(input_value: Input) -> object:
    value = input_value["value"]
    match input_value["kind"]:
        case "user":
            return user_output(value)
        case "group":
            team = LiteLLM_TeamTable.model_validate(value)
            with patch.object(proxy_server, "prisma_client", object()):
                group = asyncio.run(
                    ScimTransformations.transform_litellm_team_to_scim_group(team)
                )
            return group.model_dump()
