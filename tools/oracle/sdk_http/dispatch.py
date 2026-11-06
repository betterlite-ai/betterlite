import contextlib
import sys

from openai_chat import ChatCall, Outcome, run_chat
from pydantic import BaseModel, ConfigDict

MAX_REQUEST_BYTES = 1048576


class RequestTooLargeError(ValueError):
    def __init__(self) -> None:
        super().__init__("the call is larger than the request limit")


class OpenaiChatCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    openai_chat: ChatCall


class Request(BaseModel):
    model_config = ConfigDict(extra="forbid")

    call: OpenaiChatCall
    api_base: str


def main() -> None:
    raw = sys.stdin.buffer.read(MAX_REQUEST_BYTES + 1)
    if len(raw) > MAX_REQUEST_BYTES:
        raise RequestTooLargeError
    request = Request.model_validate_json(raw)
    with contextlib.redirect_stdout(sys.stderr):
        outcome: Outcome = run_chat(request.call.openai_chat, request.api_base)
    sys.stdout.write(outcome.model_dump_json(by_alias=True, exclude_unset=True))


if __name__ == "__main__":
    main()
