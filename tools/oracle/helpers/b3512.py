import base64
import logging
from datetime import UTC, datetime
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict

import botocore.auth  # type: ignore[import-untyped]  # https://github.com/boto/botocore/blob/444e17a456ee01d28fd0a521936edf2e8da535eb/botocore/auth.py#L433
import httpx
from litellm._logging import verbose_logger
from litellm.llms.strands_decider.decisions.transformation import (
    StrandsDeciderDecisionsConfig,
    agentcore_runtime,
    invoked_runtime,
)
from litellm.types.decisions import (
    DecisionsIRAnswer,
    DecisionsIRChoiceAnswer,
    DecisionsIRPredicateAnswer,
    DecisionsIRPredicateQuestion,
    DecisionsIRRefusal,
    DecisionsIRRequest,
    DecisionsIRResponse,
    DecisionsIRScoreAnswer,
    DecisionsIRState,
    UnsupportedDecisionsRequest,
)
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

PROVIDER: Final = "strands_decider"
MODEL: Final = "m"
DROPPED_MARKER: Final = "header(s) "
NAMES: Final = TypeAdapter(list[str])

Object: TypeAlias = dict[str, JsonValue]


class RuntimeStep(TypedDict):
    kind: Literal["runtime"]
    api_base: str


class ConfigStep(TypedDict):
    kind: Literal["config"]


class SignStep(TypedDict):
    kind: Literal["sign"]
    api_base: str
    headers: NotRequired[dict[str, str]]
    api_key: NotRequired[str | None]
    litellm_params: NotRequired[Object]
    model: str
    state: str
    question: str
    instructions: str
    now: int


class TransformStep(TypedDict):
    kind: Literal["transform"]
    api_base: str
    body: str
    model: str
    state: str
    question: str
    instructions: str


Step: TypeAlias = RuntimeStep | ConfigStep | SignStep | TransformStep


class Input(TypedDict):
    steps: list[Step]


class FrozenClock:
    def __init__(self, epoch: int) -> None:
        self.moment = datetime.fromtimestamp(epoch, tz=UTC).replace(tzinfo=None)

    def __call__(self) -> datetime:
        return self.moment


class DroppedHeaders(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.names: list[str] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        _, marker, listed = record.getMessage().partition(DROPPED_MARKER)
        if marker:
            self.names.extend(NAMES.validate_json(listed.replace("'", '"')))


def ir_request(step: SignStep | TransformStep) -> DecisionsIRRequest:
    question = DecisionsIRPredicateQuestion(
        name=step["question"],
        instructions=step["instructions"],
        criteria=None,
        extra={},
    )
    return DecisionsIRRequest(input=DecisionsIRState(state=step["state"]), questions=(question,))


def runtime_step(step: RuntimeStep) -> Object:
    api_base = step["api_base"]
    config = StrandsDeciderDecisionsConfig()
    runtime = agentcore_runtime(api_base)
    complete_url = config.get_complete_url(api_base=api_base, model=MODEL)
    invoked = invoked_runtime(httpx.URL(complete_url))
    rendered: Object | None = None
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


def config_step() -> Object:
    config = StrandsDeciderDecisionsConfig()
    shown: Object = {}
    shown["api_key_env"] = [*config.api_key_env]
    shown["api_base_env"] = [*config.api_base_env]
    shown["api_key_required"] = config.api_key_required
    shown["api_key"] = config.resolve_api_key(None)
    shown["api_base"] = config.resolve_api_base(None)
    return shown


def sign_step(step: SignStep) -> Object:
    config = StrandsDeciderDecisionsConfig()
    api_base = step["api_base"]
    headers = step.get("headers")
    params = step.get("litellm_params")
    match config.transform_decisions_request(step["model"], ir_request(step), PROVIDER):
        case UnsupportedDecisionsRequest(reason=reason):
            raise ValueError(reason)
        case body:
            dropped = DroppedHeaders()
            verbose_logger.addHandler(dropped)
            botocore.auth.get_current_datetime = FrozenClock(step["now"])  # type: ignore[misc]  # https://github.com/boto/botocore/blob/444e17a456ee01d28fd0a521936edf2e8da535eb/botocore/auth.py#L433
            signed, payload = config.sign_request(
                headers={} if headers is None else headers,
                url=config.get_complete_url(api_base=api_base, model=step["model"]),
                api_base=api_base,
                body=body,
                api_key=step.get("api_key"),
                litellm_params={} if params is None else params,
            )
            return {
                "dropped": [*dropped.names],
                "headers": dict(signed),
                "body": None if payload is None else base64.b64encode(payload).decode(),
            }


def answer_projection(answer: DecisionsIRAnswer) -> Object:
    match answer:
        case DecisionsIRPredicateAnswer(probability=probability):
            return {"kind": "DecisionsIRPredicateAnswer", "probability": probability}
        case DecisionsIRChoiceAnswer():
            return {"kind": "DecisionsIRChoiceAnswer"}
        case DecisionsIRScoreAnswer():
            return {"kind": "DecisionsIRScoreAnswer"}
        case DecisionsIRRefusal():
            return {"kind": "DecisionsIRRefusal"}


def project_response(response: DecisionsIRResponse) -> Object:
    usage = response.usage
    return {
        "model": response.model,
        "answers": [answer_projection(answer) for answer in response.answers],
        "usage": {
            "input_tokens": usage.input_tokens,
            "output_tokens": usage.output_tokens,
            "cached_tokens": usage.cached_tokens,
            "cache_write_tokens": usage.cache_write_tokens,
        },
    }


def transform_step(step: TransformStep) -> Object:
    config = StrandsDeciderDecisionsConfig()
    url = config.get_complete_url(api_base=step["api_base"], model=step["model"])
    request = httpx.Request("POST", url)
    response = httpx.Response(200, content=step["body"].encode(), request=request)
    answered = ir_request(step)
    result = config.transform_decisions_response(step["model"], PROVIDER, response, answered)
    return project_response(result)


def run(input_value: Input) -> list[Object]:
    results: list[Object] = []
    for step in input_value["steps"]:
        if step["kind"] == "runtime":
            results.append(runtime_step(step))
        elif step["kind"] == "config":
            results.append(config_step())
        elif step["kind"] == "sign":
            results.append(sign_step(step))
        else:
            results.append(transform_step(step))
    return results
