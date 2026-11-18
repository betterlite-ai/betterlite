from typing import ClassVar

from not_built import Unbuilt


class ResponseChatDispatchRequest(Unbuilt):
    task: ClassVar[str] = "B921"

    response_chat_dispatch: None
