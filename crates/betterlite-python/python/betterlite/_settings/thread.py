import threading
from collections.abc import Mapping


class MyLocal(threading.local):
    def __init__(self) -> None:
        self.user = "Hello World"


_thread_context = MyLocal()


def identify(event_details: Mapping[str, str]) -> None:
    if "user" in event_details:
        _thread_context.user = event_details["user"]


__all__ = [
    "MyLocal", "_thread_context", "identify",
]
