from typing import Final, NoReturn

from pydantic import TypeAdapter
from typing_extensions import TypedDict

from betterlite._native import NativeA2aError
from betterlite.a2a_protocol.exceptions import (
    A2AAgentCardDiscoveryError,
    A2AAgentCardError,
    A2AConnectionError,
    A2AError,
    A2ALocalhostURLError,
)


class KindPayload(TypedDict):
    type: str


class MessagePayload(TypedDict):
    message: str


class DetailedPayload(TypedDict):
    message: str
    status_code: int
    llm_provider: str
    model: str | None
    litellm_debug_info: str | None
    max_retries: int | None
    num_retries: int | None


class ConnectionPayload(TypedDict):
    message: str
    url: str | None
    model: str | None
    litellm_debug_info: str | None
    max_retries: int | None
    num_retries: int | None


class AgentCardPayload(TypedDict):
    message: str
    status_code: int
    url: str | None
    model: str | None
    litellm_debug_info: str | None


class FailurePayload(TypedDict):
    path: str
    error: str


class DiscoveryPayload(TypedDict):
    message: str
    status_code: int
    base_url: str
    failures: list[FailurePayload]


class LocalhostPayload(TypedDict):
    localhost_url: str
    base_url: str
    model: str | None
    original_error: str | None


KIND: Final[TypeAdapter[KindPayload]] = TypeAdapter(KindPayload)
MESSAGE: Final[TypeAdapter[MessagePayload]] = TypeAdapter(MessagePayload)
DETAILED: Final[TypeAdapter[DetailedPayload]] = TypeAdapter(DetailedPayload)
CONNECTION: Final[TypeAdapter[ConnectionPayload]] = TypeAdapter(ConnectionPayload)
AGENT_CARD: Final[TypeAdapter[AgentCardPayload]] = TypeAdapter(AgentCardPayload)
DISCOVERY: Final[TypeAdapter[DiscoveryPayload]] = TypeAdapter(DiscoveryPayload)
LOCALHOST: Final[TypeAdapter[LocalhostPayload]] = TypeAdapter(LocalhostPayload)

# B8081's NativeA2aError JSON names the LiteLLM class in `type`, so this maps that name to it
A2A_ERROR_TYPES: Final[dict[str, type[BaseException]]] = {
    "A2AError": A2AError,
    "A2AConnectionError": A2AConnectionError,
    "A2AAgentCardError": A2AAgentCardError,
    "A2AAgentCardDiscoveryError": A2AAgentCardDiscoveryError,
    "A2ALocalhostURLError": A2ALocalhostURLError,
    "ValueError": ValueError,
    "RuntimeError": RuntimeError,
}


class UnknownNativeError(Exception):
    payload: str

    def __init__(self, payload: str) -> None:
        self.payload = payload
        super().__init__(payload)


class UnsupportedA2AArgument(Exception):  # noqa: N818  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/a2a_protocol/main.py#L470-L500
    argument: str

    def __init__(self, argument: str) -> None:
        self.argument = argument
        super().__init__(
            f"{argument} is not supported by betterlite: the A2A call runs in Rust"
        )


# Each class reads only the payload keys its own constructor takes
def _build_error(kind: str, text: str) -> BaseException | None:
    error: BaseException | None = None
    match kind:
        case "A2AError":
            detailed = DETAILED.validate_json(text)
            error = A2AError(
                detailed["message"],
                status_code=detailed["status_code"],
                llm_provider=detailed["llm_provider"],
                model=detailed["model"],
                litellm_debug_info=detailed["litellm_debug_info"],
                max_retries=detailed["max_retries"],
                num_retries=detailed["num_retries"],
            )
        case "A2AConnectionError":
            connection = CONNECTION.validate_json(text)
            error = A2AConnectionError(
                connection["message"],
                url=connection["url"],
                model=connection["model"],
                litellm_debug_info=connection["litellm_debug_info"],
                max_retries=connection["max_retries"],
                num_retries=connection["num_retries"],
            )
        case "A2AAgentCardError":
            card = AGENT_CARD.validate_json(text)
            error = A2AAgentCardError(
                card["message"],
                url=card["url"],
                model=card["model"],
                litellm_debug_info=card["litellm_debug_info"],
                status_code=card["status_code"],
            )
        case "A2AAgentCardDiscoveryError":
            discovery = DISCOVERY.validate_json(text)
            failures = tuple(
                (failure["path"], RuntimeError(failure["error"]))
                for failure in discovery["failures"]
            )
            error = A2AAgentCardDiscoveryError(
                discovery["base_url"],
                failures,
                discovery["status_code"],
            )
        case "A2ALocalhostURLError":
            localhost = LOCALHOST.validate_json(text)
            original = localhost["original_error"]
            error = A2ALocalhostURLError(
                localhost["localhost_url"],
                localhost["base_url"],
                None if original is None else RuntimeError(original),
                model=localhost["model"],
            )
        case "ValueError":
            error = ValueError(MESSAGE.validate_json(text)["message"])
        case "RuntimeError":
            error = RuntimeError(MESSAGE.validate_json(text)["message"])
        case object():
            error = None
    return error


def raise_native_error(error: NativeA2aError) -> NoReturn:
    text = str(error)
    kind = KIND.validate_json(text)["type"]
    if kind not in A2A_ERROR_TYPES:
        raise UnknownNativeError(text) from error
    built = _build_error(kind, text)
    if built is None:
        raise UnknownNativeError(text) from error
    raise built from error
