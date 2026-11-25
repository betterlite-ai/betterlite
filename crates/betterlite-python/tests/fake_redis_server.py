import socket
import socketserver
import threading
import time
from collections.abc import Callable, Iterator
from io import BufferedReader
from typing import Final, TypeAlias

import pytest
from typing_extensions import override

_HOST: Final = "127.0.0.1"
_INFO: Final = b"redis_version:7.2.0\r\n"
# redis-py 8 talks RESP3 and expects this map back from the HELLO handshake.
_HELLO: Final = (
    b"%3\r\n"
    b"$6\r\nserver\r\n$5\r\nredis\r\n"
    b"$7\r\nversion\r\n$5\r\n7.2.0\r\n"
    b"$5\r\nproto\r\n:3\r\n"
)
_NIL: Final = b"$-1\r\n"
_OK: Final = b"+OK\r\n"
_PONG: Final = b"+PONG\r\n"
_PING_ARGUMENTS: Final = 1
_SET_ARGUMENTS: Final = 3
_POLL_INTERVAL: Final = 0.01


def _bulk(payload: bytes | None) -> bytes:
    if payload is None:
        return _NIL
    return b"$" + str(len(payload)).encode() + b"\r\n" + payload + b"\r\n"


def _read_command(reader: BufferedReader) -> list[bytes] | None:
    header = reader.readline().strip()
    if not header:
        return None
    if not header.startswith(b"*"):
        return header.split()
    count = int(header[1:])
    args: list[bytes] = []
    for _index in range(count):
        size_line = reader.readline().strip()
        if not size_line.startswith(b"$"):
            return None
        size = int(size_line[1:])
        payload = reader.read(size + 2)
        if len(payload) < size:
            return None
        args.append(payload[:size])
    return args


def _set_ttl(options: list[bytes]) -> float | None:
    for index, option in enumerate(options):
        if option.upper() == b"EX" and index + 1 < len(options):
            return float(int(options[index + 1]))
    return None


# The in-process RESP2 server LiteLLM and betterlite share, the same idea as the Rust FakeRedis.
class FakeRedisServer(socketserver.ThreadingTCPServer):
    daemon_threads = True
    allow_reuse_address = True

    def __init__(self) -> None:
        super().__init__((_HOST, 0), socketserver.BaseRequestHandler)
        self.host: str = _HOST
        self.port: int = self.server_address[1]
        self._lock = threading.Lock()
        self._values: dict[str, tuple[bytes, float | None]] = {}
        self._unknown: list[str] = []
        self._connections: list[socket.socket] = []
        self._stopped = threading.Event()
        self._thread: threading.Thread | None = None

    def start(self) -> None:
        self._thread = threading.Thread(target=self.serve_forever, args=(_POLL_INTERVAL,))
        self._thread.start()

    def stop(self) -> None:
        self._stopped.set()
        self.shutdown()
        self.server_close()
        with self._lock:
            connections = list(self._connections)
            self._connections.clear()
        for connection in connections:
            connection.close()
        if self._thread is not None:
            self._thread.join(timeout=5.0)

    @property
    def unknown_commands(self) -> list[str]:
        with self._lock:
            return list(self._unknown)

    def stored(self, key: str) -> bytes | None:
        with self._lock:
            entry = self._values.get(key)
        return None if entry is None else entry[0]

    @override
    def finish_request(
        self,
        request: socket.socket | tuple[bytes, socket.socket],
        client_address: tuple[str, int],
    ) -> None:
        _unread = client_address
        match request:
            case socket.socket() as connection:
                with self._lock:
                    self._connections.append(connection)
                try:
                    self._serve(connection)
                finally:
                    with self._lock:
                        if connection in self._connections:
                            self._connections.remove(connection)
            case tuple():
                return

    @staticmethod
    def _error(text: str) -> bytes:
        return b"-ERR " + text.encode() + b"\r\n"

    @staticmethod
    def _integer(value: int) -> bytes:
        return b":" + str(value).encode() + b"\r\n"

    @staticmethod
    def _array(items: list[bytes]) -> bytes:
        return b"*" + str(len(items)).encode() + b"\r\n" + b"".join(items)

    def reply(self, args: list[bytes]) -> bytes:
        name = args[0].upper() if args else b""
        handler = _COMMANDS.get(name)
        if handler is None:
            with self._lock:
                self._unknown.append(name.decode())
            return self._error(f"unknown command '{name.decode()}'")
        return handler(self, args)

    def ping(self, args: list[bytes]) -> bytes:
        return _PONG if len(args) == _PING_ARGUMENTS else _bulk(args[1])

    def accept(self, _args: list[bytes]) -> bytes:
        return _OK

    def hello(self, _args: list[bytes]) -> bytes:
        return _HELLO

    def info(self, _args: list[bytes]) -> bytes:
        return _bulk(_INFO)

    def get(self, args: list[bytes]) -> bytes:
        entry = self._entry(args[1].decode())
        return _bulk(None if entry is None else entry[0])

    def set(self, args: list[bytes]) -> bytes:
        if len(args) < _SET_ARGUMENTS:
            return self._error("wrong number of arguments for 'set'")
        key = args[1].decode()
        exists = self._entry(key) is not None
        if b"NX" in {option.upper() for option in args[3:]} and exists:
            return _NIL
        ttl = _set_ttl(args[3:])
        expire_at = None if ttl is None else time.monotonic() + ttl
        with self._lock:
            self._values[key] = (args[2], expire_at)
        return _OK

    def mget(self, args: list[bytes]) -> bytes:
        return self._array([self.get([b"GET", key]) for key in args[1:]])

    def incrby(self, args: list[bytes]) -> bytes:
        try:
            amount = int(args[2])
        except ValueError:
            return self._error("value is not an integer or out of range")
        return self._bump(args[1].decode(), amount, amount)

    def incrbyfloat(self, args: list[bytes]) -> bytes:
        try:
            amount = float(args[2])
        except ValueError:
            return self._error("value is not a valid float")
        return self._bump(args[1].decode(), amount, None)

    def ttl(self, args: list[bytes]) -> bytes:
        entry = self._entry(args[1].decode())
        if entry is None:
            return self._integer(-2)
        if entry[1] is None:
            return self._integer(-1)
        return self._integer(max(0, int(entry[1] - time.monotonic())))

    def expire(self, args: list[bytes]) -> bytes:
        key = args[1].decode()
        entry = self._entry(key)
        if entry is None:
            return self._integer(0)
        with self._lock:
            self._values[key] = (entry[0], time.monotonic() + int(args[2]))
        return self._integer(1)

    def delete(self, args: list[bytes]) -> bytes:
        removed = 0
        for key in args[1:]:
            name = key.decode()
            if self._entry(name) is not None:
                with self._lock:
                    del self._values[name]
                removed += 1
        return self._integer(removed)

    def flushall(self, _args: list[bytes]) -> bytes:
        with self._lock:
            self._values.clear()
        return _OK

    def _entry(self, key: str) -> tuple[bytes, float | None] | None:
        now = time.monotonic()
        with self._lock:
            entry = self._values.get(key)
            if entry is None:
                return None
            if entry[1] is not None and entry[1] <= now:
                del self._values[key]
                return None
            return entry

    def _bump(self, key: str, amount: float, whole: int | None) -> bytes:
        entry = self._entry(key)
        try:
            current = 0.0 if entry is None else float(entry[0])
        except ValueError:
            return self._error("value is not a number")
        payload = (
            str(whole + int(current)).encode()
            if whole is not None
            else str(current + amount).encode()
        )
        with self._lock:
            expire_at = None if entry is None else entry[1]
            self._values[key] = (payload, expire_at)
        return self._integer(int(payload)) if whole is not None else _bulk(payload)

    def _serve(self, connection: socket.socket) -> None:
        reader = connection.makefile("rb")
        with reader:
            while not self._stopped.is_set():
                args = _read_command(reader)
                if args is None:
                    return
                try:
                    connection.sendall(self.reply(args))
                except OSError:
                    return


# Every module that needs the fake takes this one fixture, so LiteLLM and betterlite share a server.
@pytest.fixture(autouse=False)
def redis_server() -> Iterator[FakeRedisServer]:
    server = FakeRedisServer()
    server.start()
    yield server
    server.stop()
    assert server.unknown_commands == []


_Command: TypeAlias = Callable[[FakeRedisServer, list[bytes]], bytes]
_COMMANDS: Final[dict[bytes, _Command]] = {
    b"PING": FakeRedisServer.ping,
    b"INFO": FakeRedisServer.info,
    b"CLIENT": FakeRedisServer.accept,
    b"HELLO": FakeRedisServer.hello,
    b"SELECT": FakeRedisServer.accept,
    b"AUTH": FakeRedisServer.accept,
    b"SET": FakeRedisServer.set,
    b"GET": FakeRedisServer.get,
    b"MGET": FakeRedisServer.mget,
    b"INCRBY": FakeRedisServer.incrby,
    b"INCRBYFLOAT": FakeRedisServer.incrbyfloat,
    b"EXPIRE": FakeRedisServer.expire,
    b"TTL": FakeRedisServer.ttl,
    b"DEL": FakeRedisServer.delete,
    b"FLUSHALL": FakeRedisServer.flushall,
}
