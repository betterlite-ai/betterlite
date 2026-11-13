import json
from collections.abc import Mapping
from typing import Literal, NotRequired, TypedDict, cast

import litellm
from litellm.llms.codestral.completion.handler import TextCompletionCodestralError
from litellm.types.utils import TextCompletionResponse

# ruff: noqa: SLF001
# pyright: reportPrivateUsage=false, reportUnknownMemberType=false, reportUnknownVariableType=false

Object = dict[str, object]


class Settings(TypedDict, total=False):
    suffix: str
    temperature: int
    top_p: float
    max_tokens: int
    min_tokens: int
    stream: bool
    random_seed: int
    stop: str


class OptionsStep(TypedDict):
    kind: Literal["options"]
    model: str
    params: list[tuple[str, object]]
    optional_params: NotRequired[Object]


class ReplyStep(TypedDict):
    kind: Literal["reply"]
    status: int
    body: str
    messages: NotRequired[list[object]]
    api_key: NotRequired[str]
    request_data: NotRequired[object]


class ChunkStep(TypedDict):
    kind: Literal["chunk"]
    line: str


class Input(TypedDict):
    steps: list[OptionsStep | ReplyStep | ChunkStep]
    settings: NotRequired[Settings]


class FixtureResponse:
    def __init__(self, status_code: int, text: str) -> None:
        self.status_code = status_code
        self.text = text

    def json(self) -> object:
        return cast(object, json.loads(self.text))


class FixtureLogger:
    def __init__(self) -> None:
        self.events: list[Object] = []

    def post_call(self, **params: object) -> None:
        additional = cast(Object, params.get("additional_args"))
        self.events.append({
            "name": "post_call",
            "input": params.get("input"),
            "api_key": params.get("api_key"),
            "original_response": params.get("original_response"),
            "complete_input_dict": additional.get("complete_input_dict"),
        })


def ignore_verbose(*_args: object) -> None:
    pass


def pick(source: Mapping[str, object], names: list[str]) -> Object:
    return {name: source.get(name) for name in names}


def reply_step(step: ReplyStep) -> Object:
    logger = FixtureLogger()
    try:
        result = litellm.CodestralTextCompletion().process_text_completion_response(
            model="codestral-latest",
            response=FixtureResponse(step["status"], step["body"]),  # pyright: ignore[reportArgumentType]
            model_response=TextCompletionResponse(),
            stream=False,
            logging_obj=logger,  # pyright: ignore[reportArgumentType]
            optional_params={},
            api_key=step.get("api_key", ""),
            data=cast(Object, step.get("request_data")),
            messages=step.get("messages", []),
            print_verbose=ignore_verbose,
            encoding=None,
        )
    except TextCompletionCodestralError as error:
        outcome: Object = {
            "kind": "error",
            "status_code": error.status_code,
            "request": {"method": error.request.method, "url": str(error.request.url)},
            "response_status": error.response.status_code,
        }
        return {"events": logger.events, "outcome": outcome}
    dumped = cast(Object, result.model_dump())
    reply = pick(dumped, ["id", "object", "created", "model"])
    reply["usage"] = pick(
        cast(Object, dumped["usage"]), ["prompt_tokens", "completion_tokens", "total_tokens"]
    )
    reply["choices"] = [
        pick(choice, ["index", "text", "finish_reason", "logprobs"])
        for choice in cast(list[Object], dumped["choices"])
    ]
    return {"events": logger.events, "outcome": {"kind": "reply", "reply": reply}}


def run(data: Input) -> object:
    config = litellm.CodestralTextCompletionConfig(**data.get("settings", {}))
    results: list[Object] = []
    for step in data["steps"]:
        if step["kind"] == "reply":
            results.append(reply_step(step))
        elif step["kind"] == "chunk":
            parsed = config._chunk_parser(step["line"])
            results.append(pick(parsed, ["text", "is_finished", "finish_reason", "logprobs"]))
        else:
            model = step["model"]
            mapped = config.map_openai_params(
                dict(step["params"]), step.get("optional_params", {}), model, drop_params=False
            )
            results.append({
                "model": model,
                "config": config.get_config(),
                "supported": config.get_supported_openai_params(model),
                "mapped": mapped,
            })
    return results
