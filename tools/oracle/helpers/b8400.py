import copy
import json
import sys
from collections.abc import Callable
from datetime import date
from types import ModuleType
from typing import TypeAlias, cast
from unittest.mock import patch

from fastapi import HTTPException
from litellm.proxy import utils as proxy_utils

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Fields: TypeAlias = dict[str, Json]


class FrozenDate(date):
    fixed: date | None = None

    @classmethod
    def today(cls) -> "FrozenDate":
        fixed = cls.fixed
        if fixed is None:
            raise RuntimeError("no frozen date")
        return cls(fixed.year, fixed.month, fixed.day)


class PlainError(Exception):
    pass


class MessageHTTPException(HTTPException):
    message: Json

    def __init__(self, status_code: int, detail: Json, message: Json) -> None:
        super().__init__(status_code=status_code, detail=detail)
        self.message = message


def float_text(value: float) -> str:
    return repr(value)


def standard_json(text: str) -> str:
    return json.dumps(json.loads(text), sort_keys=True, separators=(",", ":"))


def field(call: Fields, name: str) -> Json:
    value = call.get(name)
    if value is None:
        raise ValueError(f"{name} is required")
    return value


def proxy_state(
    *, premium_user: bool, prisma_client: object | None
) -> ModuleType:
    # _premium_user_check and get_prisma_client_or_throw read these module globals at call time
    module = ModuleType("litellm.proxy.proxy_server")
    module.__dict__["premium_user"] = premium_user
    module.__dict__["prisma_client"] = prisma_client
    return module


def error_message(call: Fields) -> Json:
    detail = call.get("detail")
    message = call.get("message")
    status = cast(int, field(call, "status"))
    # an empty list stands for a detail that is neither text nor a map, so LiteLLM reads the message
    exc: HTTPException = HTTPException(
        status_code=status,
        detail=[] if detail is None else cast(Json, detail),
    )
    if message is not None:
        exc = MessageHTTPException(status_code=status, detail=exc.detail, message=message)
    text = proxy_utils.get_error_message_str(exc)
    if isinstance(detail, dict) or isinstance(message, dict):
        return standard_json(text)
    return text


def plain_error_message(call: Fields) -> Json:
    return proxy_utils.get_error_message_str(PlainError(cast(str, field(call, "text"))))


def team_models(call: Fields) -> Json:
    team_config = copy.deepcopy(call.get("team_config"))
    request_data = copy.deepcopy(call.get("request_data"))
    result = "ok"
    try:
        proxy_utils._is_valid_team_configs(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
            call.get("team_id"), team_config, request_data
        )
    except Exception as error:  # the case compares which refusal LiteLLM reports
        if not str(error).startswith("Invalid model for team"):
            raise
        result = "invalid_model"
    return {"result": result, "team_config": team_config}


def month_end(call: Fields) -> Json:
    return proxy_utils._get_month_end_date(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        date.fromisoformat(cast(str, field(call, "today")))
    ).isoformat()


def projection(call: Fields) -> Json:
    spend = float(cast(str, field(call, "spend")))
    soft_limit_value = call.get("soft_limit")
    soft_limit = None if soft_limit_value is None else float(cast(str, soft_limit_value))
    FrozenDate.fixed = date.fromisoformat(cast(str, field(call, "today")))
    with patch.object(proxy_utils, "date", FrozenDate):
        over = proxy_utils._is_projected_spend_over_limit(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
            spend, soft_limit
        )
        reached = cast(
            "tuple[float, date] | None",
            proxy_utils._get_projected_spend_over_limit(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
                spend, soft_limit
            ),
        )
    limit: Json = None
    if reached is not None:
        limit = {"projected_spend": float_text(reached[0]), "date": reached[1].isoformat()}
    return {"over_limit": over, "limit": limit}


def premium(call: Fields) -> Json:
    module = proxy_state(
        premium_user=cast(bool, field(call, "premium_user")), prisma_client=None
    )
    with patch.dict(sys.modules, {module.__name__: module}):
        try:
            proxy_utils._premium_user_check(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                cast("str | None", call.get("feature"))
            )
        except HTTPException as error:
            return {"allowed": False, "status": error.status_code, "detail": error.detail}
    return {"allowed": True}


def database(call: Fields) -> Json:
    module = proxy_state(
        premium_user=False,
        prisma_client=object() if cast(bool, field(call, "connected")) else None,
    )
    with patch.dict(sys.modules, {module.__name__: module}):
        try:
            proxy_utils.get_prisma_client_or_throw(cast(str, field(call, "message")))
        except HTTPException as error:
            return {"connected": False, "status": error.status_code, "detail": error.detail}
    return {"connected": True}


CALLS: dict[str, Callable[[Fields], Json]] = {
    "error_message": error_message,
    "plain_error_message": plain_error_message,
    "team_models": team_models,
    "month_end": month_end,
    "projection": projection,
    "premium": premium,
    "database": database,
}


def run(input_value: Fields) -> Json:
    calls = cast("list[Fields]", input_value["calls"])
    return {"results": [CALLS[cast(str, call["do"])](call) for call in calls]}
