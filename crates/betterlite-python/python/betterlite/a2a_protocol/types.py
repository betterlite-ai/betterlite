from collections.abc import Mapping
from typing import Literal, Self

from pydantic import JsonValue, PrivateAttr

from betterlite.types.utils.base import ResponseObject


# LiteLLM's a2a.types.SendMessageRequest, built from a plain dict so no a2a package is needed
class SendMessageRequest(ResponseObject):
    id: str | int
    jsonrpc: Literal["2.0"] = "2.0"
    method: Literal["message/send"] = "message/send"
    params: dict[str, JsonValue]


class SendStreamingMessageRequest(ResponseObject):
    id: str | int
    jsonrpc: Literal["2.0"] = "2.0"
    method: Literal["message/stream"] = "message/stream"
    params: dict[str, JsonValue]


class SendStreamingMessageResponse(ResponseObject):
    id: str | int | None = None
    jsonrpc: str = "2.0"
    result: dict[str, JsonValue] | None = None
    error: dict[str, JsonValue] | None = None


class LiteLLMSendMessageResponse(ResponseObject):
    id: str | int | None = None
    jsonrpc: str = "2.0"
    result: dict[str, JsonValue] | None = None
    error: dict[str, JsonValue] | None = None
    usage: dict[str, JsonValue] | None = None
    _hidden_params: dict[str, JsonValue] = PrivateAttr(default_factory=dict)

    @classmethod
    def from_dict(cls, data: Mapping[str, JsonValue], request_id: JsonValue = None) -> Self:
        return cls.model_validate(_normalize_jsonrpc_response(data, request_id))


# LiteLLM's _normalize_a2a_jsonrpc_response: bool ids become text, absent ones take request_id
def _normalize_jsonrpc_response(
    response: Mapping[str, JsonValue], request_id: JsonValue
) -> dict[str, JsonValue]:
    normalized = dict(response)
    match normalized.get("id"):
        case bool() as flag:
            normalized["id"] = str(flag)
        case None if request_id is not None:
            match request_id:
                case bool() as flag:
                    normalized["id"] = str(flag)
                case str() as text:
                    normalized["id"] = text
                case int() as number:
                    normalized["id"] = number
                case object():
                    normalized["id"] = str(request_id)
        case object():
            pass
    return normalized
