from typing import ClassVar

from not_built import Unbuilt


class SpeechChatBridgeRequest(Unbuilt):
    task: ClassVar[str] = "B1003"

    speech_chat_bridge: None
