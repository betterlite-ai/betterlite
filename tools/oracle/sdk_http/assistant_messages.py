from typing import ClassVar

from not_built import Unbuilt


class AssistantMessagesRequest(Unbuilt):
    task: ClassVar[str] = "B1057"

    assistant_messages: None
