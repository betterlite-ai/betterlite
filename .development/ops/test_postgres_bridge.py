import os
import socket
import subprocess
import tempfile
import threading
import unittest
from pathlib import Path
from typing import cast

from environment import PYTHON

SCRIPT = Path(__file__).with_name("postgres_bridge.py")


class BridgeTests(unittest.TestCase):
    def test_toy_database_is_reachable_and_relay_dies_with_the_command(self) -> None:
        with socket.socket() as server, tempfile.TemporaryDirectory() as temporary:
            server.bind(("127.0.0.1", 0))
            server.listen()
            server.settimeout(10)
            port = cast(tuple[str, int], server.getsockname())[1]

            def answer() -> None:
                # The first connection is the transport health probe.
                for _ in range(2):
                    connection = server.accept()[0]
                    with connection:
                        request = connection.recv(100)
                        if request:
                            connection.sendall(request.upper())

            thread = threading.Thread(target=answer)
            thread.start()
            record = Path(temporary) / "socket-path"
            child = (
                "import os,socket,sys; from pathlib import Path; "
                "from urllib.parse import urlsplit,parse_qs; "
                "u=urlsplit(os.environ['POSTGRES_TEST_URL']); "
                "p=Path(parse_qs(u.query)['host'][0])/('.s.PGSQL.'+str(u.port)); "
                "Path(sys.argv[1]).write_text(str(p)); "
                "s=socket.socket(socket.AF_UNIX); s.connect(str(p)); "
                "s.sendall(b'select silly goose'); "
                "assert s.recv(100)==b'SELECT SILLY GOOSE'"
            )
            result = subprocess.run(
                [
                    *PYTHON,
                    str(SCRIPT),
                    "--",
                    "bwrap",
                    "--unshare-net",
                    "--ro-bind",
                    "/",
                    "/",
                    "--dev",
                    "/dev",
                    "--bind",
                    temporary,
                    temporary,
                    "--setenv",
                    "UV_CACHE_DIR",
                    temporary + "/uv-cache",
                    "--",
                    *PYTHON[:2],
                    "--no-sync",
                    *PYTHON[2:],
                    "-c",
                    child,
                    str(record),
                ],
                env={**os.environ, "POSTGRES_TEST_URL": f"postgresql://fixture:fixture@127.0.0.1:{port}/toy"},
                capture_output=True,
                text=True,
                check=False,
                timeout=15,
            )
            thread.join(timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertFalse(Path(record.read_text()).exists())

    def test_an_unreachable_database_stops_before_starting_the_child(self) -> None:
        with socket.socket() as reserved, tempfile.TemporaryDirectory() as temporary:
            reserved.bind(("127.0.0.1", 0))
            port = cast(tuple[str, int], reserved.getsockname())[1]
            marker = Path(temporary) / "worker-started"
            result = subprocess.run(
                [*PYTHON, str(SCRIPT), "--", "touch", str(marker)],
                env={**os.environ, "POSTGRES_TEST_URL": f"postgresql://127.0.0.1:{port}/toy"},
                capture_output=True,
                text=True,
                check=False,
                timeout=10,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(marker.exists())


if __name__ == "__main__":
    unittest.main()
