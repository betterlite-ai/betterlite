import logging
import os
from collections.abc import Iterator
from contextlib import contextmanager

from litellm.exceptions import UnsupportedParamsError
from litellm.llms.bedrock_mantle.responses.transformation import BedrockMantleResponsesAPIConfig
from litellm.types.router import GenericLiteLLMParams
from pydantic import JsonValue
from typing_extensions import override

from .models import ENVIRONMENT_KEYS, MapCase, Object, UrlCase

BODY_INPUT = "hello"


def config_of(case: UrlCase | MapCase) -> BedrockMantleResponsesAPIConfig:
    use_openai_path = case.get("use_openai_path")
    given = True if use_openai_path is None else use_openai_path
    return BedrockMantleResponsesAPIConfig(use_openai_path=given)


def transform_body(
    config: BedrockMantleResponsesAPIConfig, model: str, source: JsonValue, params: Object
) -> Object:
    extra = GenericLiteLLMParams()
    body: Object = config.transform_responses_api_request(model, source, params, extra, {})  # type: ignore[arg-type]  # pyright: ignore[reportUnknownMemberType, reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock_mantle/responses/transformation.py#L183-L189
    return body


def map_result(
    config: BedrockMantleResponsesAPIConfig, case: MapCase, *, with_body: bool
) -> Object:
    params = case.get("params")
    drop_params = case.get("drop_params")
    given = {} if params is None else params
    drop = False if drop_params is None else drop_params
    model = case["model"]
    try:
        mapped: Object = config.map_openai_params(given, model, drop)  # type: ignore[arg-type]  # pyright: ignore[reportUnknownMemberType, reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock_mantle/responses/transformation.py#L227-L232
        body = transform_body(config, model, BODY_INPUT, mapped) if with_body else None
    except UnsupportedParamsError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L945
        return {"error": {"status": error.status_code}}
    if body is None:
        return {"params": mapped}
    return {"body": body}


class Warnings(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.messages: list[JsonValue] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())


@contextmanager
def warnings_recorded() -> Iterator[list[JsonValue]]:
    recorder = Warnings()
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(recorder)
    try:
        yield recorder.messages
    finally:
        logger.removeHandler(recorder)


@contextmanager
def environment(values: dict[str, str] | None) -> Iterator[None]:
    given = {} if values is None else values
    saved = {name: os.environ.pop(name, None) for name in ENVIRONMENT_KEYS}
    os.environ.update({name: given[name] for name in ENVIRONMENT_KEYS if name in given})
    try:
        yield
    finally:
        for name in ENVIRONMENT_KEYS:
            os.environ.pop(name, None)
        os.environ.update({name: value for name, value in saved.items() if value is not None})
