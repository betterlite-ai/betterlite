import contextlib
import os
import select
import signal
import socket
import socketserver
import subprocess
import sys
import tempfile
import threading
from pathlib import Path
from typing import cast
from urllib.parse import parse_qsl, urlencode, urlsplit, urlunsplit


class FixtureUnavailable(RuntimeError):
    """The configured fixture cannot be reached before starting paid work."""


def run(command: list[str], url: str) -> int:
    parsed = urlsplit(url)
    if parsed.hostname not in ("127.0.0.1", "localhost", "::1"):
        raise FixtureUnavailable("POSTGRES_TEST_URL must name the loopback fixture")
    endpoint = (parsed.hostname, parsed.port or 5432)
    try:
        with socket.create_connection(endpoint, timeout=5):
            pass
    except OSError as error:
        raise FixtureUnavailable("test database unavailable; no worker started") from error
    slots = threading.BoundedSemaphore(64)

    class Forward(socketserver.BaseRequestHandler):
        def handle(self) -> None:
            if not slots.acquire(blocking=False):
                return
            client = cast(socket.socket, self.request)
            try:
                client.settimeout(5)
                with socket.create_connection(endpoint, timeout=5) as upstream:
                    peers = (client, upstream)
                    while readable := select.select(peers, [], [], 300)[0]:
                        for source in readable:
                            data = source.recv(65536)
                            if not data:
                                return
                            destination = upstream if source is client else client
                            destination.sendall(data)
            except OSError:
                return
            finally:
                slots.release()

    # A short name avoids Linux's 108-byte Unix socket path limit. Mode 0700 confines access.
    with tempfile.TemporaryDirectory(prefix="bl-pg-", dir="/tmp") as directory:
        path = str(Path(directory) / f".s.PGSQL.{endpoint[1]}")
        with socketserver.ThreadingUnixStreamServer(path, Forward) as server:
            server.daemon_threads = True
            threading.Thread(target=server.serve_forever, daemon=True).start()
            query = [(key, value) for key, value in parse_qsl(parsed.query) if key != "host"]
            query.append(("host", directory))
            environment = {
                **os.environ,
                "POSTGRES_TEST_URL": urlunsplit(parsed._replace(query=urlencode(query))),
                "GATE_POSTGRES_SOCKET": path,
            }
            child = subprocess.Popen(
                ["setpriv", "--pdeathsig", "TERM", "--", *command],
                env=environment,
                start_new_session=True,
            )

            def terminate(signum: int, _frame: object) -> None:
                with contextlib.suppress(ProcessLookupError):
                    os.killpg(child.pid, signum)

            signal.signal(signal.SIGTERM, terminate)
            signal.signal(signal.SIGINT, terminate)
            try:
                return child.wait()
            finally:
                terminate(signal.SIGKILL, None)
                child.wait()
                server.shutdown()


if __name__ == "__main__":
    arguments = sys.argv[1:]
    if arguments[:1] == ["--"]:
        arguments = arguments[1:]
    if not arguments:
        sys.exit("usage: postgres_bridge.py -- COMMAND [ARG ...]")
    try:
        sys.exit(run(arguments, os.environ["POSTGRES_TEST_URL"]))
    except (FixtureUnavailable, KeyError) as error:
        sys.exit(str(error))
