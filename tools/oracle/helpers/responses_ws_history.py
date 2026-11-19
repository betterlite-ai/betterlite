import asyncio
from datetime import datetime
from typing import Final, TypedDict

import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.responses.streaming_iterator import ManagedResponsesWebSocketHandler as Handler
from pydantic import JsonValue, StrictInt, TypeAdapter

litellm.suppress_debug_info = True

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
# the handler's helpers take object-valued maps, so a case's JSON map crosses the boundary here
OBJECTS: Final = TypeAdapter(dict[str, object])
OBJECT_LIST: Final = TypeAdapter(list[dict[str, object]])
WARMUP_ID_PREFIX: Final = "resp_warmup_"
SERIALIZED: Final[dict[str, JsonValue]] = {"json": {"kind": "json", "n": 7}, "text": "text",
                                           "number": 7, "null": None, "bool": True, "list": [1, 2]}


Setup = TypedDict("Setup", {
    "model": str, "custom_llm_provider": str | None, "api_key": str | None, "api_base": str | None,
    "timeout": float | None, "first_message": str | None,
    "litellm_metadata": dict[str, JsonValue] | None, "kwargs": dict[str, JsonValue],
})


# _build_warmup_response promises an int timestamp, and StrictInt makes the adapter enforce it
class WarmupResponse(TypedDict):
    id: str
    created_at: StrictInt
    object: str
    status: str
    model: JsonValue
    output: list[JsonValue]
    usage: dict[str, JsonValue]


class WarmupEvent(TypedDict):
    type: str
    response: WarmupResponse


WARMUP_RESPONSE: Final = TypeAdapter(WarmupResponse)
WARMUP_EVENT: Final = TypeAdapter(WarmupEvent)


InjectRow = TypedDict("InjectRow", {"kwargs": dict[str, JsonValue], "model": str | None})
ErrorRow = TypedDict("ErrorRow", {"message": str, "error_type": str | None, "fail": bool})
ParseRow = TypedDict("ParseRow", {"raw": str, "fail": bool})
StoreRow = TypedDict("StoreRow", {"id": str, "messages": list[dict[str, JsonValue]]})
HistoryCase = TypedDict("HistoryCase", {
    "store": list[StoreRow], "first": str, "append": dict[str, JsonValue], "missing": str,
})
ApplyRow = TypedDict("ApplyRow", {
    "previous_response_id": str | None, "current": list[dict[str, JsonValue]],
    "prior": list[dict[str, JsonValue]],
})


class Input(TypedDict):
    setup: Setup
    resolve: list[str | None]
    inject: list[InjectRow]
    proxy: list[dict[str, JsonValue]]
    serialize: list[str]
    errors: list[ErrorRow]
    parse: list[ParseRow]
    warmup: list[dict[str, JsonValue]]
    warmup_ids: list[str | None]
    inputs: list[JsonValue]
    completions: list[dict[str, JsonValue]]
    history: HistoryCase
    base_kwargs: list[dict[str, JsonValue]]
    apply: list[ApplyRow]


class Socket:
    def __init__(self) -> None:
        self.frames: list[str] = []
        self.fail_next = False

    async def send_text(self, data: str) -> None:
        await asyncio.sleep(0)
        if self.fail_next:
            self.fail_next = False
            raise RuntimeError("the case asks the socket to refuse the frame")
        self.frames.append(data)

    async def receive_text(self) -> str:
        await asyncio.sleep(0)
        return ""


class Probe(Handler):
    def constructor_view(self) -> JsonValue:
        return JSON.validate_python({
            "model": self.model,
            "model_group": self.model_group,
            "connection_provider": self._connection_provider,
            "litellm_metadata": dict(self.litellm_metadata),
            "api_key": self.api_key,
            "api_base": self.api_base,
            "timeout": self.timeout,
            "custom_llm_provider": self.custom_llm_provider,
            "first_message": self.first_message,
            "quota_callbacks": len(self.quota_callbacks),
            "extra_kwargs": self.extra_kwargs,
            "history_empty": len(self._get_history_messages("resp_none")) == 0,
        })

    def resolve_view(self, models: list[str | None]) -> JsonValue:
        return JSON.validate_python([
            {"model": model, "provider": self._resolve_provider(model), "same": self._same_provider(model)}
            for model in models
        ])

    def inject_view(self, rows: list[InjectRow]) -> JsonValue:
        answers: list[JsonValue] = []
        for row in rows:
            kwargs = OBJECTS.validate_python(row["kwargs"])
            self._inject_credentials(kwargs, row["model"])
            answers.append(JSON.validate_python(kwargs))
        return JSON.validate_python(answers)

    def proxy_view(self, rows: list[dict[str, JsonValue]]) -> JsonValue:
        answers: list[JsonValue] = []
        for row in rows:
            kwargs = OBJECTS.validate_python(row)
            self._update_proxy_request(kwargs, self.model)
            answers.append(JSON.validate_python(kwargs))
        return JSON.validate_python(answers)

    def serialize_view(self, kinds: list[str]) -> JsonValue:
        return JSON.validate_python([self._serialize_chunk(SERIALIZED[kind]) for kind in kinds])

    async def error_view(self, socket: Socket, rows: list[ErrorRow]) -> JsonValue:
        answers: list[JsonValue] = []
        for row in rows:
            start = len(socket.frames)
            socket.fail_next = row["fail"]
            if row["error_type"] is None:
                await self._send_error(row["message"])
            else:
                await self._send_error(row["message"], row["error_type"])
            answers.append([JSON.validate_json(frame) for frame in socket.frames[start:]])
        return JSON.validate_python(answers)

    async def parse_view(self, socket: Socket, rows: list[ParseRow]) -> JsonValue:
        answers: list[JsonValue] = []
        for row in rows:
            start = len(socket.frames)
            socket.fail_next = row["fail"]
            message = await self._parse_message(row["raw"])
            frames = [JSON.validate_json(frame) for frame in socket.frames[start:]]
            if message is None:
                answers.append({"kind": "ignored", "message": None, "frames": frames})
            else:
                answers.append({"kind": "message", "message": JSON.validate_python(message), "frames": frames})
        return JSON.validate_python(answers)

    async def warmup_view(self, socket: Socket, input_value: Input) -> JsonValue:
        frames = [OBJECTS.validate_python(frame) for frame in input_value["warmup"]]
        acks: list[JsonValue] = []
        for frame in frames:
            start = len(socket.frames)
            await self._send_warmup_ack(frame)
            events = [WARMUP_EVENT.validate_json(text) for text in socket.frames[start:]]
            acks.append([
                {"type": event["type"], "response": self.response_view(event["response"])}
                for event in events
            ])
        responses = [
            self.response_view(WARMUP_RESPONSE.validate_python(self._build_warmup_response(frame)))
            for frame in frames
        ]
        return JSON.validate_python({
            "flags": [self._is_warmup_frame(frame) for frame in frames],
            "ids": [self._is_warmup_response_id(one) for one in input_value["warmup_ids"]],
            "sources": [self._warmup_source_params(frame) for frame in frames],
            "responses": responses,
            "acks": acks,
        })

    def response_view(self, response: WarmupResponse) -> JsonValue:
        identifier = response["id"]
        return JSON.validate_python({
            "id_is_warmup": identifier.startswith(WARMUP_ID_PREFIX),
            "id_hex": len(identifier) == len(WARMUP_ID_PREFIX) + 32,
            "object": response["object"],
            "created_at_is_int": True,
            "status": response["status"],
            "model": response["model"],
            "output": response["output"],
            "usage": response["usage"],
        })

    def inputs_view(self, values: list[JsonValue]) -> JsonValue:
        return JSON.validate_python([self._input_to_messages(value) for value in values])

    def completions_view(self, rows: list[dict[str, JsonValue]]) -> JsonValue:
        answers: list[JsonValue] = []
        for row in rows:
            completed = OBJECTS.validate_python(row)
            answers.append({
                "id": self._extract_response_id(completed),
                "messages": JSON.validate_python(self._extract_output_messages(completed)),
            })
        return JSON.validate_python(answers)

    def history_view(self, history: HistoryCase) -> JsonValue:
        for stored in history["store"]:
            self._store_history(stored["id"], OBJECT_LIST.validate_python(stored["messages"]))
        first = self._get_history_messages(history["first"])
        _appended = list(first)
        _appended.append(OBJECTS.validate_python(history["append"]))
        again = self._get_history_messages(history["first"])
        return JSON.validate_python({
            "first": first,
            "after_append": again,
            "missing": self._get_history_messages(history["missing"]),
        })

    def base_kwargs_view(self, rows: list[dict[str, JsonValue]]) -> JsonValue:
        return JSON.validate_python([
            self._build_base_call_kwargs(OBJECTS.validate_python(row)) for row in rows
        ])

    def apply_view(self, rows: list[ApplyRow]) -> JsonValue:
        answers: list[JsonValue] = []
        for row in rows:
            kwargs = OBJECTS.validate_python({})
            self._apply_history(
                kwargs,
                row["previous_response_id"],
                OBJECT_LIST.validate_python(row["current"]),
                OBJECT_LIST.validate_python(row["prior"]),
            )
            answers.append(JSON.validate_python(kwargs))
        return JSON.validate_python(answers)

    async def answers(self, socket: Socket, input_value: Input) -> JsonValue:
        return JSON.validate_python({
            "constructor": self.constructor_view(),
            "resolve": self.resolve_view(input_value["resolve"]),
            "inject": self.inject_view(input_value["inject"]),
            "proxy": self.proxy_view(input_value["proxy"]),
            "serialize": self.serialize_view(input_value["serialize"]),
            "errors": await self.error_view(socket, input_value["errors"]),
            "parse": await self.parse_view(socket, input_value["parse"]),
            "warmup": await self.warmup_view(socket, input_value),
            "inputs": self.inputs_view(input_value["inputs"]),
            "completions": self.completions_view(input_value["completions"]),
            "history": self.history_view(input_value["history"]),
            "base_kwargs": self.base_kwargs_view(input_value["base_kwargs"]),
            "apply": self.apply_view(input_value["apply"]),
        })


def new_probe(setup: Setup) -> tuple[Probe, Socket]:
    socket = Socket()
    logging_object = Logging(
        model=setup["model"],
        messages=None,
        stream=True,
        call_type="responses",
        start_time=datetime.fromisoformat("2026-01-01T00:00:00+00:00"),
        litellm_call_id="call-1",
        function_id="fn-1",
    )
    handler = Probe(
        websocket=socket,
        model=setup["model"],
        logging_obj=logging_object,
        litellm_metadata=setup["litellm_metadata"],
        api_key=setup["api_key"],
        api_base=setup["api_base"],
        timeout=setup["timeout"],
        custom_llm_provider=setup["custom_llm_provider"],
        first_message=setup["first_message"],
        **setup["kwargs"],
    )
    return handler, socket


def run(input_value: Input) -> JsonValue:
    handler, socket = new_probe(input_value["setup"])
    return asyncio.run(handler.answers(socket, input_value))
