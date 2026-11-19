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
MESSAGES: Final = TypeAdapter(list[dict[str, object]])
EVENT: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
WARMUP_PREFIX: Final = "resp_warmup_"


class Setup(TypedDict):
    model: str
    custom_llm_provider: str | None
    api_key: str | None
    api_base: str | None
    timeout: float | None
    first_message: str | None
    litellm_metadata: dict[str, JsonValue] | None
    kwargs: dict[str, JsonValue]


class WarmupResponse(TypedDict):
    id: str
    created_at: StrictInt
    object: str
    status: str
    model: JsonValue
    output: list[JsonValue]
    usage: dict[str, JsonValue]


class CredentialRow(TypedDict):
    model: str | None
    kwargs: dict[str, JsonValue]


class FrameRow(TypedDict):
    kind: str
    payload: str
    error_type: str | None
    fail: bool


class StoreRow(TypedDict):
    id: str
    messages: list[dict[str, JsonValue]]


class ApplyRow(TypedDict):
    previous_response_id: str | None
    current: list[dict[str, JsonValue]]
    prior: list[dict[str, JsonValue]]


class Case(TypedDict):
    inputs: list[JsonValue]
    completions: list[dict[str, JsonValue]]
    store: list[StoreRow]
    first: str
    append: dict[str, JsonValue]
    missing: str
    applied: list[ApplyRow]
    base_kwargs: list[dict[str, JsonValue]]


class Input(TypedDict):
    setup: Setup
    credentials: list[CredentialRow]
    proxy: list[dict[str, JsonValue]]
    frames: list[FrameRow]
    warmup: list[dict[str, JsonValue]]
    warmup_ids: list[str | None]
    case: Case


WARMUP_RESPONSE: Final[TypeAdapter[WarmupResponse]] = TypeAdapter(WarmupResponse)


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
    def constructor_view(self) -> dict[str, JsonValue]:
        view: dict[str, JsonValue] = {"model": self.model, "model_group": self.model_group}
        view["connection_provider"] = self._connection_provider
        view["litellm_metadata"] = JSON.validate_python(self.litellm_metadata)
        view["api_key"] = self.api_key
        view["api_base"] = self.api_base
        view["timeout"] = self.timeout
        view["custom_llm_provider"] = self.custom_llm_provider
        view["first_message"] = self.first_message
        view["quota_callbacks"] = len(self.quota_callbacks)
        view["extra_kwargs"] = JSON.validate_python(self.extra_kwargs)
        view["history_empty"] = not self._get_history_messages("resp_none")
        return view

    def credentials_view(self, rows: list[CredentialRow]) -> list[JsonValue]:
        answers: list[JsonValue] = []
        for row in rows:
            model = row["model"]
            kwargs = OBJECTS.validate_python(row["kwargs"])
            self._inject_credentials(kwargs, model)
            answers.append({"model": model, "provider": self._resolve_provider(model),
                            "same": self._same_provider(model),
                            "kwargs": JSON.validate_python(kwargs)})
        return answers

    def proxy_rows(self, rows: list[dict[str, JsonValue]]) -> list[JsonValue]:
        answers: list[JsonValue] = []
        for row in rows:
            kwargs = OBJECTS.validate_python(row)
            self._update_proxy_request(kwargs, self.model)
            answers.append(JSON.validate_python(kwargs))
        return answers

    async def frame_row(self, socket: Socket, row: FrameRow) -> JsonValue:
        start = len(socket.frames)
        socket.fail_next = row["fail"]
        if row["kind"] == "serialize":
            chunk = JSON.validate_json(row["payload"])
            return JSON.validate_python(self._serialize_chunk(chunk))
        if row["kind"] == "error":
            error_type = row["error_type"]
            if error_type is None:
                await self._send_error(row["payload"])
            else:
                await self._send_error(row["payload"], error_type)
            frames = [JSON.validate_json(text) for text in socket.frames[start:]]
            return JSON.validate_python(frames)
        return await self.parse_row(socket, row, start)

    async def parse_row(self, socket: Socket, row: FrameRow, start: int) -> JsonValue:
        message = await self._parse_message(row["payload"])
        frames = [JSON.validate_json(text) for text in socket.frames[start:]]
        if message is None:
            return JSON.validate_python({"kind": "ignored", "message": None, "frames": frames})
        message_view = JSON.validate_python(message)
        return JSON.validate_python({"kind": "message", "message": message_view, "frames": frames})

    def ack_event(self, event: dict[str, JsonValue]) -> dict[str, JsonValue]:
        response = WARMUP_RESPONSE.validate_python(event["response"])
        return {"type": event["type"], "response": self.response_view(response)}

    def response_view(self, response: WarmupResponse) -> dict[str, JsonValue]:
        identifier = response["id"]
        return {"id_is_warmup": identifier.startswith(WARMUP_PREFIX),
                "id_hex": len(identifier) == len(WARMUP_PREFIX) + 32,
                "object": response["object"], "status": response["status"],
                "model": response["model"], "output": response["output"],
                "usage": response["usage"]}

    async def warmup_view(self, socket: Socket, input_value: Input) -> dict[str, JsonValue]:
        frames = [OBJECTS.validate_python(frame) for frame in input_value["warmup"]]
        acks: list[JsonValue] = []
        flags: list[JsonValue] = []
        sources: list[JsonValue] = []
        ids: list[JsonValue] = []
        for frame in frames:
            start = len(socket.frames)
            await self._send_warmup_ack(frame)
            acks.append([self.ack_event(EVENT.validate_json(text))
                         for text in socket.frames[start:]])
            flags.append(self._is_warmup_frame(frame))
            sources.append(JSON.validate_python(self._warmup_source_params(frame)))
        for one in input_value["warmup_ids"]:
            ids.append(JSON.validate_python(self._is_warmup_response_id(one)))
        built = [WARMUP_RESPONSE.validate_python(self._build_warmup_response(frame))
                 for frame in frames]
        responses: list[JsonValue] = [self.response_view(response) for response in built]
        return {"flags": flags, "ids": ids, "sources": sources,
                "responses": responses, "acks": acks}

    def messages_view(self, case: Case) -> dict[str, JsonValue]:
        inputs: list[JsonValue] = [
            JSON.validate_python(self._input_to_messages(value)) for value in case["inputs"]
        ]
        completions: list[JsonValue] = []
        for row in case["completions"]:
            completed = OBJECTS.validate_python(row)
            completions.append({"id": self._extract_response_id(completed),
                                "messages": JSON.validate_python(
                                    self._extract_output_messages(completed))})
        return {"inputs": inputs, "completions": completions}

    def history_view(self, case: Case) -> dict[str, JsonValue]:
        for stored in case["store"]:
            self._store_history(stored["id"], MESSAGES.validate_python(stored["messages"]))
        first = self._get_history_messages(case["first"])
        _appended = list(first)
        _appended.append(OBJECTS.validate_python(case["append"]))
        again = self._get_history_messages(case["first"])
        applied: list[JsonValue] = []
        for row in case["applied"]:
            kwargs = OBJECTS.validate_python({})
            self._apply_history(kwargs, row["previous_response_id"],
                                MESSAGES.validate_python(row["current"]),
                                MESSAGES.validate_python(row["prior"]))
            applied.append(JSON.validate_python(kwargs))
        base: list[JsonValue] = [
            JSON.validate_python(self._build_base_call_kwargs(OBJECTS.validate_python(row)))
            for row in case["base_kwargs"]
        ]
        return {"first": JSON.validate_python(first),
                "after_append": JSON.validate_python(again),
                "missing": JSON.validate_python(self._get_history_messages(case["missing"])),
                "applied": applied, "base_kwargs": base}

    async def answers(self, socket: Socket, input_value: Input) -> dict[str, JsonValue]:
        views: dict[str, JsonValue] = {"constructor": self.constructor_view()}
        views["credentials"] = self.credentials_view(input_value["credentials"])
        views["proxy"] = self.proxy_rows(input_value["proxy"])
        frames: list[JsonValue] = []
        for row in input_value["frames"]:
            frames.append(await self.frame_row(socket, row))
        views["frames"] = frames
        views["warmup"] = await self.warmup_view(socket, input_value)
        views["messages"] = self.messages_view(input_value["case"])
        views["history"] = self.history_view(input_value["case"])
        return views


def new_probe(setup: Setup) -> tuple[Probe, Socket]:
    socket = Socket()
    extra = OBJECTS.validate_python(setup["kwargs"])
    _ = extra.pop("user_api_key_dict", None)
    log_obj = Logging(model=setup["model"], messages=None, stream=True, call_type="responses",
                      start_time=datetime.fromisoformat("2026-01-01T00:00:00+00:00"),
                      litellm_call_id="call-1", function_id="fn-1")
    return Probe(websocket=socket, model=setup["model"], logging_obj=log_obj,
                 litellm_metadata=setup["litellm_metadata"], api_key=setup["api_key"],
                 api_base=setup["api_base"], timeout=setup["timeout"],
                 custom_llm_provider=setup["custom_llm_provider"],
                 first_message=setup["first_message"], user_api_key_dict=None,
                 quota_callbacks=None, **extra), socket


def run(input_value: Input) -> dict[str, JsonValue]:
    handler, socket = new_probe(input_value["setup"])
    return asyncio.run(handler.answers(socket, input_value))
