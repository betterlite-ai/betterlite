from typing import ClassVar

from not_built import Unbuilt


class AssistantCreateRequest(Unbuilt):
    task: ClassVar[str] = "B1055"

    assistant_create: None
