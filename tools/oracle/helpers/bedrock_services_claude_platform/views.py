import json
import logging
import os
from collections.abc import Generator, Mapping
from contextlib import contextmanager

from litellm.types.utils import ModelResponse
from pydantic import JsonValue as Json
from typing_extensions import override

from .types import (
    AUTHENTICATION_PREFIX,
    BETA_HEADER,
    BILLING_MARKER,
    CHUNK_VIEW,
    DROP_MARKER,
    EMPTY_PARAMS,
    IGNORE_MARKER,
    MANAGED_ENV,
    SYSTEM_BLOCKS,
    WORKSPACE_HEADER,
    Failure,
    WorkspaceFields,
)


class Warnings(logging.Handler):
    def __init__(self) -> None:
        super().__init__()
        self.entries: list[Json] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.entries.append(warning_entry(record.getMessage()))


@contextmanager
def captured_warnings() -> Generator[list[Json], None, None]:
    handler = Warnings()
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(handler)
    try:
        yield handler.entries
    finally:
        logger.removeHandler(handler)


def warning_entry(message: str) -> Json:
    if DROP_MARKER in message:
        _, _, listed = message.partition(DROP_MARKER)
        tuple_text, _, _ = listed.partition(" from the request body")
        inner = tuple_text.strip()[1:-1].rstrip(",")
        names: list[Json] = (
            [] if not inner else [name.strip("'\"") for name in inner.split(", ")]
        )
        return {"action": "drop", "params": names}
    _, _, type_text = message.partition(IGNORE_MARKER)
    return {"action": "ignore", "type_name": type_text.split(";", 1)[0]}


def failure_view(error: Exception) -> Json:
    failure = Failure.model_validate(error, from_attributes=True)
    model = failure.model if failure.model is not None else ""
    return {
        "status": failure.status_code,
        "provider": failure.llm_provider,
        "model": model,
        "message": failure.message.removeprefix(AUTHENTICATION_PREFIX),
    }


def apply_environment(environment: dict[str, str] | None) -> None:
    # Each case's workspace values replace the previous case's, so no value leaks between them.
    for name in MANAGED_ENV:
        os.environ.pop(name, None)
    if environment is not None:
        os.environ.update(environment)


def merged_params(case: WorkspaceFields) -> dict[str, Json]:
    merged: dict[str, Json] = {}
    settings = case.get("litellm_settings")
    if settings is not None:
        merged.update(dict[str, Json](settings))
    deployment = case.get("deployment")
    if deployment is not None:
        merged.update(dict[str, Json](deployment))
    return merged


def params_of(optional: dict[str, Json] | None) -> dict[str, Json]:
    if optional is None:
        return EMPTY_PARAMS
    return optional


def beta_tokens(value: str) -> list[Json]:
    tokens: set[str] = {piece.strip() for piece in value.split(",") if piece.strip()}
    ordered: list[Json] = []
    ordered.extend(sorted(tokens))
    return ordered


def beta_view(headers: Mapping[str, str]) -> list[Json]:
    value = headers.get(BETA_HEADER)
    if value is None:
        return []
    return beta_tokens(value)


def project_headers(headers: Mapping[str, str]) -> list[Json]:
    projected: list[Json] = []
    for name in sorted(headers):
        value: Json = headers[name]
        if name == BETA_HEADER:
            value = beta_tokens(headers[name])
        pair: list[Json] = [name, value]
        projected.append(pair)
    return projected


def system_texts(system: Json) -> list[Json]:
    if system is None:
        return []
    texts: list[Json] = []
    for block in SYSTEM_BLOCKS.validate_python(system):
        if block["type"] != "text":
            continue
        text = block.get("text")
        if text is not None:
            texts.append(text)
    return texts


def chunk_view(item: ModelResponse) -> Json:
    view = CHUNK_VIEW.validate_json(item.model_dump_json())
    model = view["model"] if view["model"] is not None else ""
    choices = view["choices"]
    if not choices:
        return {"id": view["id"], "model": model, "content": "", "finish_reason": None}
    choice = choices[0]
    content = choice["delta"]["content"] if choice["delta"]["content"] is not None else ""
    return {
        "id": view["id"],
        "model": model,
        "content": content,
        "finish_reason": choice["finish_reason"],
    }


def body_view(
    body: dict[str, Json], warnings: list[Json], headers: Mapping[str, str]
) -> Json:
    return {
        "model": body.get("model"),
        "max_tokens": body.get("max_tokens"),
        "workspace_removed": "workspace_id" not in body,
        "context_removed": "context_management" not in body,
        "override_removed": "claude_platform_unsupported_params" not in body,
        "system_texts": system_texts(body.get("system")),
        "billing": BILLING_MARKER in json.dumps(body),
        "beta": beta_view(headers),
        "warnings": warnings,
    }


def workspace_view(headers: Mapping[str, str]) -> Json:
    workspace: Json = headers[WORKSPACE_HEADER]
    return {"workspace": workspace}
