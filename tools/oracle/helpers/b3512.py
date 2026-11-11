import base64
from collections.abc import Callable, Generator, Mapping
from contextlib import contextmanager
from datetime import UTC, datetime
from typing import Final, TypeAlias, cast
from unittest.mock import patch

import botocore.auth
import httpx
from litellm._logging import verbose_logger
from litellm.llms.strands_decider.decisions.transformation import (
    StrandsDeciderDecisionsConfig,
    agentcore_runtime,
    invoked_runtime,
)
from litellm.types.decisions import (
    DecisionsIRPredicateAnswer,
    DecisionsIRPredicateQuestion,
    DecisionsIRRequest,
    DecisionsIRResponse,
    DecisionsIRState,
    UnsupportedDecisionsRequest,
)
from pydantic import JsonValue

Json: TypeAlias = JsonValue
Object: TypeAlias = dict[str, Json]

PROVIDER: Final = "strands_decider"
MODEL: Final = "m"


def obj(value: Json) -> Object:
    if not isinstance(value, dict):
        raise TypeError("expected an object")
    return value


def obj_or_empty(value: Json | None) -> Object:
    return {} if value is None else obj(value)


def text(fields: Object, name: str) -> str:
    value = fields.get(name)
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def optional_text(fields: Object, name: str) -> str | None:
    value = fields.get(name)
    if value is not None and not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def expect_int(fields: Object, name: str) -> int:
    value = fields.get(name)
    if isinstance(value, bool) or not isinstance(value, int):
        raise TypeError(f"{name} must be an integer")
    return value


def headers_of(fields: Object, name: str) -> dict[str, str]:
    value = fields.get(name)
    if value is None:
        return {}
    headers: dict[str, str] = {}
    for key, item in obj(value).items():
        if not isinstance(item, str):
            raise TypeError(f"{name} values must be strings")
        headers[key] = item
    return headers


def ir_request(fields: Object) -> DecisionsIRRequest:
    return DecisionsIRRequest(
        input=DecisionsIRState(state=text(fields, "state")),
        questions=(
            DecisionsIRPredicateQuestion(
                name=text(fields, "question"),
                instructions=text(fields, "instructions"),
                criteria=None,
                extra={},
            ),
        ),
    )


@contextmanager
def frozen_clock(epoch: int) -> Generator[None, None, None]:
    frozen = datetime.fromtimestamp(epoch, tz=UTC).replace(tzinfo=None)
    with patch.object(botocore.auth, "get_current_datetime", lambda: frozen):
        yield


@contextmanager
def recorded_dropped() -> Generator[list[Json], None, None]:
    dropped: list[Json] = []

    def capture(_message: str, *args: object, **_kwargs: object) -> None:
        if args:
            dropped.extend(cast("list[Json]", args[0]))

    with patch.object(verbose_logger, "warning", capture):
        yield dropped


def runtime_step(fields: Object) -> Json:
    api_base = text(fields, "api_base")
    config = StrandsDeciderDecisionsConfig()
    runtime = agentcore_runtime(api_base)
    complete_url = config.get_complete_url(api_base=api_base, model=MODEL)
    invoked = invoked_runtime(httpx.URL(complete_url))
    rendered: Json = None
    if runtime is not None:
        rendered = {
            "arn": runtime.arn,
            "region": runtime.region,
            "url": runtime.invocations_url,
            "default_session_id": runtime.default_session_id,
        }
    return {
        "runtime": rendered,
        "signs_with_aws": config.signs_with_aws(api_base),
        "complete_url": complete_url,
        "invoked_region": None if invoked is None else invoked.region,
    }


def config_step(_fields: Object) -> Json:
    config = StrandsDeciderDecisionsConfig()
    return {
        "api_key_env": cast("Json", list(config.api_key_env)),
        "api_base_env": cast("Json", list(config.api_base_env)),
        "api_key_required": config.api_key_required,
        "api_key": config.resolve_api_key(None),
        "api_base": config.resolve_api_base(None),
    }


def sign_step(fields: Object) -> Json:
    config = StrandsDeciderDecisionsConfig()
    api_base = text(fields, "api_base")
    model = text(fields, "model")
    headers = headers_of(fields, "headers")
    api_key = optional_text(fields, "api_key")
    params: Mapping[str, object] = obj_or_empty(fields.get("litellm_params"))
    body = config.transform_decisions_request(model, ir_request(fields), PROVIDER)
    if isinstance(body, UnsupportedDecisionsRequest):
        raise ValueError(body.reason)
    url = config.get_complete_url(api_base=api_base, model=model)
    with frozen_clock(expect_int(fields, "now")), recorded_dropped() as dropped:
        signed, payload = config.sign_request(
            headers=headers,
            url=url,
            api_base=api_base,
            body=body,
            api_key=api_key,
            litellm_params=params,
        )
    forwarded = dict(signed)
    return {
        "dropped": cast("Json", list(dropped)),
        "headers": cast("Json", forwarded),
        "body": None if payload is None else base64.b64encode(payload).decode(),
    }


def answer_projection(answer: object) -> Json:
    if isinstance(answer, DecisionsIRPredicateAnswer):
        return {"kind": "DecisionsIRPredicateAnswer", "probability": answer.probability}
    return {"kind": type(answer).__name__}


def project_response(response: DecisionsIRResponse) -> Json:
    usage = response.usage
    return {
        "model": response.model,
        "answers": cast("Json", [answer_projection(answer) for answer in response.answers]),
        "usage": {
            "input_tokens": usage.input_tokens,
            "output_tokens": usage.output_tokens,
            "cached_tokens": usage.cached_tokens,
            "cache_write_tokens": usage.cache_write_tokens,
        },
    }


def transform_step(fields: Object) -> Json:
    config = StrandsDeciderDecisionsConfig()
    api_base = text(fields, "api_base")
    model = text(fields, "model")
    url = config.get_complete_url(api_base=api_base, model=model)
    body = text(fields, "body").encode()
    response = httpx.Response(200, content=body, request=httpx.Request("POST", url))
    result = config.transform_decisions_response(model, PROVIDER, response, ir_request(fields))
    return project_response(result)


HANDLERS: dict[str, Callable[[Object], Json]] = {
    "runtime": runtime_step,
    "config": config_step,
    "sign": sign_step,
    "transform": transform_step,
}


def execute(step: Object) -> Json:
    kind = text(step, "kind")
    found = HANDLERS.get(kind)
    if found is None:
        raise ValueError(f"unknown step kind {kind}")
    return found(step)


def run(value: dict[str, Json]) -> Json:
    steps = value.get("steps")
    if not isinstance(steps, list):
        raise TypeError("steps must be a list")
    return [execute(obj(step)) for step in steps]
