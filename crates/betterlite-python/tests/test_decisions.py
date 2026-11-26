import asyncio
import importlib
import inspect
import json
import threading
from collections.abc import Iterator, Sequence
from contextlib import closing, suppress
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import ClassVar, Final, Protocol

import pytest
from pydantic import JsonValue, TypeAdapter
from typing_extensions import TypedDict

from betterlite._completion.errors import UnsupportedCallArgumentError
from betterlite._native import NativeDecisionError
from betterlite.decisions import adecisions, decisions, supported_decisions_providers
from betterlite.exceptions import BadRequestError
from betterlite.sandbox.main import (
    ContainerHandle,
    acode_interpreter_tool,
    acreate_sandbox,
    adelete_sandbox,
    arun_code,
)
from betterlite.sandbox.sandbox_tools import (
    clear_sandbox_tools,
    register_sandbox_tools,
    resolve_sandbox_tool,
)
from betterlite.systemone import asystemone, systemone
from betterlite.types.decisions import (
    DecisionsResponse,
    NoulQuestion,
    OpenAIDecisionInputMessage,
    OpenAIDecisionResponse,
    OpenAIPredicateQuestion,
)

HOST: Final = "127.0.0.1"
OK: Final = 200
BAD_REQUEST: Final = 400
NOT_FOUND: Final = 404
JSON_TYPE: Final = "application/json"
DECISIONS_PATH: Final = "/v1/decisions"
SYSTEMONE_PATH: Final = "/providers/microsoft/v1/systemone"
SANDBOXES_PATH: Final = "/sandboxes"
MODEL: Final = "gpt-4o-mini"
SYSTEMONE_MODEL: Final = "systemone-1"
SANDBOX_ID: Final = "sbx-1"
INPUT_TEXT: Final = "is this a greeting?"
SAFETY_IDENTIFIER: Final = "user-1"
API_KEY: Final = "sk-test"
E2B_KEY: Final = "sk-e2b"
PROBABILITY: Final = 0.9
INPUT_TOKENS: Final = 10
OUTPUT_TOKENS: Final = 5
TOTAL_TOKENS: Final = 15
NOUL: Final = 0.75
BAD_REQUEST_TEXT: Final = "boom"
SANDBOX_DOMAIN: Final = "e2b.app"
BAD_DOMAIN: Final = "bad host"
ENVD_VALUE: Final = "envd-token"
TRAFFIC_VALUE: Final = "traffic-token"
DEFAULT_TEMPLATE: Final = "code-interpreter-v1"
DEFAULT_TIMEOUT: Final = 300
CREATE_TIMEOUT: Final = 60
PROVIDERS: Final[tuple[str, ...]] = (
    "openai",
    "openrouter",
    "azure_ai",
    "perplexity",
    "typesafe",
    "strands_decider",
    "cloudflare",
    "hosted_vllm",
)
BODY: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
PREDICATE_QUESTION: Final[dict[str, JsonValue]] = {
    "type": "predicate",
    "name": "is_greeting",
    "instructions": "Answer whether the input is a greeting.",
}
SYSTEMONE_QUESTIONS: Final[dict[str, dict[str, JsonValue]]] = {
    "greeting": {"type": "noul", "instructions": "Answer how much of a greeting this state is."},
}
SYSTEMONE_STATE: Final[dict[str, JsonValue]] = {
    "messages": [{"role": "user", "content": "hello"}],
}
DECISION_BODY: Final[dict[str, JsonValue]] = {
    "model": MODEL,
    "answers": [
        {"type": "predicate", "name": "is_greeting", "probability": PROBABILITY},
    ],
    "usage": {
        "input_tokens": INPUT_TOKENS,
        "input_tokens_details": {"cached_tokens": 0, "cache_write_tokens": 0},
        "output_tokens": OUTPUT_TOKENS,
        "output_tokens_details": {"reasoning_tokens": 0},
        "total_tokens": TOTAL_TOKENS,
    },
}
SYSTEMONE_BODY: Final[dict[str, JsonValue]] = {
    "model": SYSTEMONE_MODEL,
    "answers": {"greeting": {"type": "noul", "noul": NOUL}},
    "usage": {"input_tokens": INPUT_TOKENS, "output_tokens": OUTPUT_TOKENS},
}
CREATE_BODY: Final[dict[str, JsonValue]] = {
    "sandboxID": SANDBOX_ID,
    "domain": SANDBOX_DOMAIN,
    "envdAccessToken": ENVD_VALUE,
    "trafficAccessToken": TRAFFIC_VALUE,
}
ERROR_BODY: Final[dict[str, JsonValue]] = {
    "error": {
        "message": BAD_REQUEST_TEXT,
        "type": "invalid_request_error",
        "param": None,
        "code": None,
    },
}


class RecordedCall(TypedDict):
    command: str
    path: str
    headers: dict[str, str]
    body: dict[str, JsonValue]


class CallOptions(TypedDict):
    model: str
    api_key: str
    api_base: str
    custom_llm_provider: str


class TokensDetailsView(TypedDict):
    cached_tokens: int
    cache_write_tokens: int


class OutputDetailsView(TypedDict):
    reasoning_tokens: int


class DecisionUsageView(TypedDict):
    input_tokens: int
    input_tokens_details: TokensDetailsView
    output_tokens: int
    output_tokens_details: OutputDetailsView
    total_tokens: int


class PredicateAnswerView(TypedDict):
    type: str
    name: str | None
    probability: float


class DecisionResponseView(TypedDict):
    model: str
    answers: Sequence[PredicateAnswerView]
    usage: DecisionUsageView


class NoulAnswerView(TypedDict):
    type: str
    noul: float


class SystemOneUsageView(TypedDict):
    input_tokens: int
    output_tokens: int


class SystemOneResponseView(TypedDict):
    model: str
    answers: dict[str, NoulAnswerView]
    usage: SystemOneUsageView


class ReferenceCall(Protocol):
    def __call__(self, *args: JsonValue, **kwargs: JsonValue) -> JsonValue: ...


class ReferenceModule(Protocol):
    def __getattr__(self, name: str) -> ReferenceCall: ...


class DefaultView(Protocol):
    @property
    def default(self) -> JsonValue: ...


DECISION_VIEW: Final[TypeAdapter[DecisionResponseView]] = TypeAdapter(DecisionResponseView)
SYSTEMONE_VIEW: Final[TypeAdapter[SystemOneResponseView]] = TypeAdapter(SystemOneResponseView)
REFERENCE_DECISIONS: ReferenceModule = importlib.import_module("litellm.decisions")
REFERENCE_SYSTEMONE: ReferenceModule = importlib.import_module("litellm.systemone")
REFERENCE_SANDBOX: ReferenceModule = importlib.import_module("litellm.sandbox.main")


class RecordingServer(ThreadingHTTPServer):
    daemon_threads = True
    calls: list[RecordedCall]
    mode: str

    def __init__(self, mode: str = "ok") -> None:
        super().__init__((HOST, 0), RecordingHandler)
        self.calls = []
        self.mode = mode
        RecordingHandler.owner = self
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()

    @property
    def url(self) -> str:
        return f"http://{HOST}:{self.server_port}"

    def record(
        self, command: str, path: str, headers: dict[str, str], body: dict[str, JsonValue]
    ) -> None:
        self.calls.append({"command": command, "path": path, "headers": headers, "body": body})

    def close(self) -> None:
        self.shutdown()
        self.server_close()
        self.thread.join()


class RecordingHandler(BaseHTTPRequestHandler):
    owner: ClassVar[RecordingServer]

    def do_POST(self) -> None:
        body = self.read_body()
        self.owner.record(self.command, self.path, dict(self.headers.items()), body)
        if self.owner.mode == "reject":
            self.reply(BAD_REQUEST, ERROR_BODY)
            return
        match self.path:
            case "/code":
                self.reply(OK, {"text": "x" * (10 * 1024 * 1024)})
            case "/v1/decisions":
                self.reply(OK, DECISION_BODY)
            case "/providers/microsoft/v1/systemone":
                self.reply(OK, SYSTEMONE_BODY)
            case "/sandboxes":
                self.reply(OK, self.create_body())
            case object():
                self.reply(NOT_FOUND, {})

    def create_body(self) -> dict[str, JsonValue]:
        if self.owner.mode in {"Failed", "Stopping", "Terminated"}:
            return {"id": SANDBOX_ID, "status": {"state": self.owner.mode}}
        if self.owner.mode == "unusable-domain":
            return {**CREATE_BODY, "domain": BAD_DOMAIN}
        return CREATE_BODY

    def do_GET(self) -> None:
        self.owner.record(self.command, self.path, dict(self.headers.items()), {})
        self.reply(OK, {"status": {"state": self.owner.mode}})

    def do_DELETE(self) -> None:
        self.owner.record(self.command, self.path, dict(self.headers.items()), {})
        if self.path.endswith("/missing"):
            self.reply(NOT_FOUND, {})
            return
        self.reply(OK, {})

    def read_body(self) -> dict[str, JsonValue]:
        declared = self.headers.get("Content-Length")
        raw = b"{}" if declared is None else self.rfile.read(int(declared))
        return BODY.validate_json(raw)

    def reply(self, status: int, payload: dict[str, JsonValue]) -> None:
        encoded = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Type", JSON_TYPE)
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)


def parameter_rows(signature: inspect.Signature) -> list[tuple[str, str, str]]:
    rows: list[tuple[str, str, str]] = []
    for parameter in signature.parameters.values():
        default: DefaultView = parameter
        rows.append((parameter.name, parameter.kind.name, repr(default.default)))
    return rows


def header_of(call: RecordedCall, name: str) -> str | None:
    lowered = name.lower()
    for header, value in call["headers"].items():
        if header.lower() == lowered:
            return value
    return None


@pytest.fixture(autouse=False)
def recording() -> Iterator[RecordingServer]:
    with closing(RecordingServer()) as server:
        yield server


def assert_native_cause(error: BaseException) -> None:
    cause = error.__cause__
    assert cause is not None
    assert cause.__class__ is NativeDecisionError


def test_decisions_signature() -> None:
    assert parameter_rows(inspect.signature(decisions)) == parameter_rows(
        inspect.signature(REFERENCE_DECISIONS.decisions)
    )
    assert parameter_rows(inspect.signature(adecisions)) == parameter_rows(
        inspect.signature(REFERENCE_DECISIONS.adecisions)
    )
    assert not inspect.iscoroutinefunction(decisions)
    assert inspect.iscoroutinefunction(adecisions)


def test_systemone_signature() -> None:
    ours = (inspect.signature(systemone), inspect.signature(asystemone))
    theirs = (
        inspect.signature(REFERENCE_SYSTEMONE.systemone),
        inspect.signature(REFERENCE_SYSTEMONE.asystemone),
    )
    assert [parameter_rows(one) for one in ours] == [parameter_rows(one) for one in theirs]
    assert not inspect.iscoroutinefunction(systemone)
    assert inspect.iscoroutinefunction(asystemone)


@pytest.mark.parametrize(
    ("ours", "reference"),
    [
        (inspect.signature(acreate_sandbox), inspect.signature(REFERENCE_SANDBOX.acreate_sandbox)),
        (inspect.signature(arun_code), inspect.signature(REFERENCE_SANDBOX.arun_code)),
        (inspect.signature(adelete_sandbox), inspect.signature(REFERENCE_SANDBOX.adelete_sandbox)),
        (
            inspect.signature(acode_interpreter_tool),
            inspect.signature(REFERENCE_SANDBOX.acode_interpreter_tool),
        ),
    ],
)
def test_sandbox_signature(ours: inspect.Signature, reference: inspect.Signature) -> None:
    ours_names = tuple(ours.parameters)
    reference_names = tuple(reference.parameters)
    assert ours_names == reference_names
    assert parameter_rows(ours) == parameter_rows(reference)


def test_providers_calls_native() -> None:
    providers = supported_decisions_providers()
    assert len(set(providers)) == len(providers)
    assert providers == PROVIDERS


def test_decisions_calls_openai() -> None:
    with closing(RecordingServer()) as server:
        response = decisions(
            model=MODEL,
            input=INPUT_TEXT,
            questions=[PREDICATE_QUESTION],
            safety_identifier=SAFETY_IDENTIFIER,
            api_key=API_KEY,
            api_base=server.url,
            custom_llm_provider="openai",
        )
        call = server.calls[-1]
    view = DECISION_VIEW.validate_json(response.model_dump_json())
    assert call["path"] == DECISIONS_PATH
    assert call["body"]["model"] == MODEL
    assert call["body"]["input"] == INPUT_TEXT
    assert call["body"]["safety_identifier"] == SAFETY_IDENTIFIER
    assert view["model"] == MODEL
    assert view["answers"][0]["type"] == "predicate"
    assert view["answers"][0]["probability"] == PROBABILITY
    assert view["usage"]["input_tokens"] == INPUT_TOKENS
    assert view["usage"]["total_tokens"] == TOTAL_TOKENS


def test_adecisions_calls_openai() -> None:
    with closing(RecordingServer()) as server:
        response = asyncio.run(
            adecisions(
                model=MODEL,
                input=INPUT_TEXT,
                questions=[PREDICATE_QUESTION],
                api_key=API_KEY,
                api_base=server.url,
                custom_llm_provider="openai",
            )
        )
        call = server.calls[-1]
    view = DECISION_VIEW.validate_json(response.model_dump_json())
    assert call["path"] == DECISIONS_PATH
    assert call["body"]["model"] == MODEL
    assert view["answers"][0]["probability"] == PROBABILITY


def test_state_calls_systemone() -> None:
    with closing(RecordingServer()) as server, pytest.warns(UserWarning, match="deprecated"):
        response = decisions(
            model=SYSTEMONE_MODEL,
            state=SYSTEMONE_STATE,
            questions=SYSTEMONE_QUESTIONS,
            api_key=API_KEY,
            api_base=server.url,
            custom_llm_provider="azure_ai",
        )
    call = server.calls[-1]
    view = SYSTEMONE_VIEW.validate_json(response.model_dump_json())
    assert call["path"] == SYSTEMONE_PATH
    assert call["body"]["state"] == SYSTEMONE_STATE
    assert view["answers"]["greeting"]["noul"] == NOUL


def test_systemone_calls_azure() -> None:
    with closing(RecordingServer()) as server:
        response = systemone(
            model=SYSTEMONE_MODEL,
            state=SYSTEMONE_STATE,
            questions=SYSTEMONE_QUESTIONS,
            api_key=API_KEY,
            api_base=server.url,
            custom_llm_provider="azure_ai",
        )
        call = server.calls[-1]
    view = SYSTEMONE_VIEW.validate_json(response.model_dump_json())
    assert call["path"] == SYSTEMONE_PATH
    assert call["body"]["model"] == SYSTEMONE_MODEL
    assert view["model"] == SYSTEMONE_MODEL
    assert view["answers"]["greeting"]["type"] == "noul"
    assert view["answers"]["greeting"]["noul"] == NOUL


def test_asystemone_calls_azure() -> None:
    with closing(RecordingServer()) as server:
        response = asyncio.run(
            asystemone(
                model=SYSTEMONE_MODEL,
                state=SYSTEMONE_STATE,
                questions=SYSTEMONE_QUESTIONS,
                api_key=API_KEY,
                api_base=server.url,
                custom_llm_provider="azure_ai",
            )
        )
    view = SYSTEMONE_VIEW.validate_json(response.model_dump_json())
    assert server.calls[-1]["path"] == SYSTEMONE_PATH
    assert view["answers"]["greeting"]["noul"] == NOUL


def test_acreate_calls_e2b() -> None:
    with closing(RecordingServer()) as server:
        handle = asyncio.run(
            acreate_sandbox(
                provider="e2b",
                template="python",
                timeout=CREATE_TIMEOUT,
                allow_internet_access=False,
                api_key=E2B_KEY,
                api_base=server.url,
            )
        )
        call = server.calls[-1]
        url = server.url
    assert call["path"] == SANDBOXES_PATH
    assert call["body"]["templateID"] == "python"
    assert call["body"]["timeout"] == CREATE_TIMEOUT
    assert call["body"]["secure"] is True
    assert call["body"]["allow_internet_access"] is False
    assert header_of(call, "x-api-key") == E2B_KEY
    assert handle.id == SANDBOX_ID
    assert handle.provider == "e2b"
    assert handle.domain == SANDBOX_DOMAIN
    assert handle.hidden_params["envd_access_token"] == ENVD_VALUE
    assert handle.hidden_params["traffic_access_token"] == TRAFFIC_VALUE
    assert handle.hidden_params["api_key"] == E2B_KEY
    assert handle.hidden_params["api_base"] == url


def test_adelete_calls_e2b() -> None:
    with closing(RecordingServer()) as server:
        deleted = asyncio.run(
            adelete_sandbox(
                provider="e2b",
                container=SANDBOX_ID,
                api_key=E2B_KEY,
                api_base=server.url,
            )
        )
        call = server.calls[-1]
        missing = asyncio.run(
            adelete_sandbox(
                provider="e2b",
                container="missing",
                api_key=E2B_KEY,
                api_base=server.url,
            )
        )
    assert deleted is True
    assert missing is False
    assert call["command"] == "DELETE"
    assert call["path"] == f"{SANDBOXES_PATH}/{SANDBOX_ID}"
    assert header_of(call, "x-api-key") == E2B_KEY


def test_handle_calls_delete() -> None:
    with closing(RecordingServer()) as server:
        handle = asyncio.run(
            acreate_sandbox(provider="e2b", api_key=E2B_KEY, api_base=server.url)
        )
        deleted = asyncio.run(
            adelete_sandbox(
                provider="e2b",
                container=handle,
                api_key=E2B_KEY,
                api_base=server.url,
            )
        )
        create = server.calls[0]
        call = server.calls[-1]
    assert deleted is True
    assert create["body"]["templateID"] == DEFAULT_TEMPLATE
    assert create["body"]["timeout"] == DEFAULT_TIMEOUT
    assert create["body"]["allow_internet_access"] is True
    assert call["path"] == f"{SANDBOXES_PATH}/{SANDBOX_ID}"


def test_run_failure_calls_delete() -> None:
    with closing(RecordingServer("unusable-domain")) as server, suppress(Exception):
        asyncio.run(
            acode_interpreter_tool(
                provider="e2b", code="print(1)", api_key=E2B_KEY, api_base=server.url
            )
        )
    commands = [call["command"] for call in server.calls]
    assert commands == ["POST", "DELETE"]
    assert server.calls[-1]["path"] == f"{SANDBOXES_PATH}/{SANDBOX_ID}"


def test_sandbox_tools_calls(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("SANDBOX_TOOL_KEY", "sk-from-env")
    register_sandbox_tools(
        [
            {
                "sandbox_tool_name": "python",
                "litellm_params": {"sandbox_provider": "e2b", "api_key": E2B_KEY},
            },
            {
                "sandbox_tool_name": "env",
                "litellm_params": {
                    "sandbox_provider": "opensandbox",
                    "api_key": "os.environ/SANDBOX_TOOL_KEY",
                    "api_base": "https://os.example",
                },
            },
            {"litellm_params": {"sandbox_provider": "e2b"}},
        ]
    )
    assert resolve_sandbox_tool("python") == {
        "sandbox_provider": "e2b",
        "api_key": E2B_KEY,
        "api_base": None,
    }
    env_tool = resolve_sandbox_tool("env")
    assert env_tool is not None
    assert env_tool["api_key"] == "sk-from-env"
    assert env_tool["api_base"] == "https://os.example"
    assert resolve_sandbox_tool("missing") is None
    clear_sandbox_tools()
    assert resolve_sandbox_tool("python") is None
    assert resolve_sandbox_tool("env") is None
    clear_sandbox_tools()
    assert resolve_sandbox_tool("missing") is None


def test_bare_id_value_error() -> None:
    with pytest.raises(ValueError, match="sandbox id alone") as raised:
        asyncio.run(arun_code(provider="e2b", container=SANDBOX_ID, code="print(1)"))
    assert_native_cause(raised.value)


def test_e2b_key_value_error(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("E2B_API_KEY", raising=False)
    with pytest.raises(ValueError, match="E2B API key not set"):
        asyncio.run(acreate_sandbox(provider="e2b"))


def test_opensandbox_base_error(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.delenv("OPEN_SANDBOX_API_BASE", raising=False)
    with pytest.raises(ValueError, match="OpenSandbox api_base is required"):
        asyncio.run(acreate_sandbox(provider="opensandbox", api_key="sk-os"))


def test_unsupported_provider() -> None:
    with pytest.raises(ValueError, match="not supported for provider: nope"):
        asyncio.run(acreate_sandbox(provider="nope", api_key=API_KEY))
    with pytest.raises(ValueError, match="not supported for provider: nope"):
        asyncio.run(acode_interpreter_tool(provider="nope", code="print(1)", api_key=API_KEY))


def test_client_unsupported() -> None:
    with pytest.raises(UnsupportedCallArgumentError, match="client"):
        decisions(model=MODEL, input=INPUT_TEXT, client="http")
    with pytest.raises(UnsupportedCallArgumentError, match="shared_session"):
        asyncio.run(acreate_sandbox(provider="e2b", shared_session="session"))


def test_rejected_decision_error() -> None:
    with closing(RecordingServer("reject")) as server, pytest.raises(
        BadRequestError, match=BAD_REQUEST_TEXT
    ):
        decisions(
            model=MODEL,
            input=INPUT_TEXT,
            questions=[PREDICATE_QUESTION],
            api_key=API_KEY,
            api_base=server.url,
            custom_llm_provider="openai",
        )


def test_systemone_input_error() -> None:
    with pytest.raises(BadRequestError, match=r"litellm\.decisions") as raised:
        systemone(
            model=SYSTEMONE_MODEL,
            input=INPUT_TEXT,
            questions=SYSTEMONE_QUESTIONS,
            api_key=API_KEY,
        )
    assert "go through litellm.decisions" in str(raised.value)


def test_tool_params_value_error() -> None:
    tools: list[dict[str, JsonValue]] = [{"sandbox_tool_name": "x", "litellm_params": 1}]
    with pytest.raises(ValueError, match="litellm_params must be a mapping"):
        register_sandbox_tools(tools)


@pytest.mark.parametrize("mode", ["sync", "async"])
def test_models_calls_openai(mode: str, recording: RecordingServer) -> None:
    message = OpenAIDecisionInputMessage(content="hi")
    question = OpenAIPredicateQuestion(
        type="predicate", name="is_greeting", instructions="Greeting?"
    )
    options: CallOptions = {
        "model": MODEL,
        "api_key": API_KEY,
        "api_base": recording.url,
        "custom_llm_provider": "openai",
    }
    if mode == "sync":
        response = decisions(input=[message], questions=[question], **options)
    else:
        response = asyncio.run(adecisions(input=[message], questions=[question], **options))
    assert recording.calls[-1]["body"]["input"] == [
        {"role": "user", "type": "message", "content": "hi"},
    ]
    assert recording.calls[-1]["body"]["questions"] == [
        {"type": "predicate", "name": "is_greeting", "instructions": "Greeting?"},
    ]
    assert DECISION_VIEW.validate_json(response.model_dump_json())["model"] == MODEL


@pytest.mark.parametrize("mode", ["sync", "async", "legacy", "legacy-async"])
def test_models_calls_systemone(mode: str, recording: RecordingServer) -> None:
    question_fields: dict[str, JsonValue] = {
        "type": "noul", "instructions": "Greeting?", "provider_hint": None,
    }
    question = TypeAdapter(NoulQuestion).validate_python(question_fields)
    questions = {"greeting": question}
    options: CallOptions = {
        "model": SYSTEMONE_MODEL,
        "api_key": API_KEY,
        "api_base": recording.url,
        "custom_llm_provider": "azure_ai",
    }
    response: DecisionsResponse | OpenAIDecisionResponse
    if mode == "sync":
        response = systemone(state=SYSTEMONE_STATE, questions=questions, **options)
    elif mode == "async":
        response = asyncio.run(
            asystemone(state=SYSTEMONE_STATE, questions=questions, **options)
        )
    elif mode == "legacy":
        with pytest.warns(UserWarning, match="deprecated"):
            response = decisions(state=SYSTEMONE_STATE, questions=questions, **options)
    else:
        with pytest.warns(UserWarning, match="deprecated"):
            response = asyncio.run(
                adecisions(state=SYSTEMONE_STATE, questions=questions, **options)
            )
    assert recording.calls[-1]["body"]["questions"] == {
        "greeting": {"type": "noul", "instructions": "Greeting?", "provider_hint": None},
    }
    assert SYSTEMONE_VIEW.validate_json(response.model_dump_json())["model"] == SYSTEMONE_MODEL


def test_tool_name_value_error() -> None:
    tools: list[dict[str, JsonValue]] = [
        {"sandbox_tool_name": 1, "litellm_params": {"sandbox_provider": "e2b"}},
    ]
    with pytest.raises(ValueError, match="sandbox_tool_name") as raised:
        register_sandbox_tools(tools)
    assert_native_cause(raised.value)


def test_output_limit_value_error(recording: RecordingServer) -> None:
    handle = ContainerHandle(id=SANDBOX_ID, provider="opensandbox")
    handle.set_hidden_params({"api_base": recording.url, "execd_endpoint": recording.url})
    with pytest.raises(ValueError, match="output exceeded 10485760 bytes") as raised:
        asyncio.run(arun_code(provider="opensandbox", container=handle, code="print(1)"))
    assert_native_cause(raised.value)


@pytest.mark.parametrize(
    ("mode", "provider", "error_type", "message"),
    [
        ("Failed", "opensandbox", ValueError, f"OpenSandbox sandbox {SANDBOX_ID} entered Failed"),
        (
            "Stopping",
            "opensandbox",
            ValueError,
            f"OpenSandbox sandbox {SANDBOX_ID} entered Stopping",
        ),
        (
            "Terminated",
            "opensandbox",
            ValueError,
            f"OpenSandbox sandbox {SANDBOX_ID} entered Terminated",
        ),
        ("reject", "e2b", Exception, "API answered with status 400"),
        ("reject", "opensandbox", Exception, "API answered with status 400"),
    ],
)
def test_create_error(
    recording: RecordingServer,
    mode: str,
    provider: str,
    error_type: type[Exception],
    message: str,
) -> None:
    recording.mode = mode
    with pytest.raises(error_type, match=message) as raised:
        asyncio.run(acreate_sandbox(provider=provider, api_key=API_KEY, api_base=recording.url))
    assert raised.value.__class__ is error_type
    assert_native_cause(raised.value)
