import logging
from typing import Literal, NotRequired, TypedDict

from litellm.proxy.db.token_auth import (
    AzureEntraTokenAuth,
    DatabaseTokenAuth,
    IAMEndpoint,
    RdsIamTokenAuth,
    parse_database_token_expiration,
    parse_iam_endpoint_from_url,
)
from typing_extensions import override

Json = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class EndpointValue(TypedDict):
    host: str
    port: str
    user: str
    name: str
    schema: NotRequired[str | None]


class BuildUrlInput(TypedDict):
    function: Literal["build_url"]
    endpoint: EndpointValue
    token: str


class ParseEndpointInput(TypedDict):
    function: Literal["parse_endpoint"]
    url: str


class ExpirationInput(TypedDict):
    function: Literal["expiration"]
    kind: Literal["rds", "azure"]
    token: str


Call = BuildUrlInput | ParseEndpointInput | ExpirationInput


class BatchInput(TypedDict):
    function: Literal["batch"]
    calls: list[Call]


Input = Call | BatchInput


class LevelCapture(logging.Handler):
    def __init__(self) -> None:
        super().__init__()
        self.levels: list[str] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.levels.append(record.levelname.lower())


def unused_token_provider() -> str:
    raise RuntimeError(
        "the expiry parser never mints a token"
    )


def authentication(kind: Literal["rds", "azure"]) -> DatabaseTokenAuth:
    if kind == "rds":
        return RdsIamTokenAuth()
    return AzureEntraTokenAuth(token_provider=unused_token_provider)


def expiry_value(kind: Literal["rds", "azure"], token: str) -> Json:
    capture = LevelCapture()
    logger = logging.getLogger("LiteLLM Proxy")
    previous_level = logger.level
    previous_handlers = list(logger.handlers)
    # LiteLLM's own handler writes DEBUG records to stdout; route them to the capture instead.
    for handler in previous_handlers:
        logger.removeHandler(handler)
    logger.addHandler(capture)
    logger.setLevel(logging.DEBUG)
    try:
        expiration = parse_database_token_expiration(authentication(kind), token)
    finally:
        logger.removeHandler(capture)
        for handler in previous_handlers:
            logger.addHandler(handler)
        logger.setLevel(previous_level)
    level = capture.levels[0] if capture.levels else None
    value = None if expiration is None else expiration.isoformat()
    return {"value": value, "log_level": level}


def run_build_url(input_value: BuildUrlInput) -> Json:
    endpoint = input_value["endpoint"]
    return IAMEndpoint(
        host=endpoint["host"],
        port=endpoint["port"],
        user=endpoint["user"],
        name=endpoint["name"],
        schema=endpoint.get("schema"),
    ).build_url(input_value["token"])


def endpoint_value(endpoint: IAMEndpoint) -> Json:
    return {
        "host": endpoint.host,
        "port": endpoint.port,
        "user": endpoint.user,
        "name": endpoint.name,
        "schema": endpoint.schema,
    }


def run_call(call: Call) -> Json:
    if call["function"] == "build_url":
        return run_build_url(call)
    if call["function"] == "parse_endpoint":
        return endpoint_value(parse_iam_endpoint_from_url(call["url"]))
    return expiry_value(call["kind"], call["token"])


def scenario(call: Call) -> Json:
    try:
        value = run_call(call)
    except (ValueError, RuntimeError, OverflowError, OSError):
        return {"ok": False}
    return {"ok": True, "value": value}


def run(input_value: Input) -> Json:
    if input_value["function"] == "batch":
        return [scenario(call) for call in input_value["calls"]]
    return run_call(input_value)
