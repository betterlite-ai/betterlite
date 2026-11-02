import subprocess
import tempfile
import threading
import unittest
from pathlib import Path

from isolate import isolate
from test_ops import git, repository


class SandboxTests(unittest.TestCase):
    def test_private_git_survives_other_checkout_metadata_changing(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source, task = root / "source", root / "B123"
            repository(source)
            isolate(source, task, "task/B123-goose")
            failures: list[str] = []

            def churn() -> None:
                try:
                    for number in range(10):
                        other = root / "other"
                        git(source, "worktree", "add", "-qb", f"other-{number}", str(other))
                        git(source, "worktree", "remove", str(other))
                except subprocess.CalledProcessError as error:
                    failures.append(str(error))

            thread = threading.Thread(target=churn)
            thread.start()
            try:
                for _ in range(10):
                    result = subprocess.run(
                        [
                            "bwrap",
                            "--unshare-net",
                            "--ro-bind",
                            "/",
                            "/",
                            "--dev",
                            "/dev",
                            "--bind",
                            str(task),
                            str(task),
                            "--",
                            "git",
                            "-C",
                            str(task),
                            "status",
                            "--porcelain",
                        ],
                        capture_output=True,
                        text=True,
                        check=False,
                        timeout=5,
                    )
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(result.stdout, "")
            finally:
                thread.join(timeout=5)
            self.assertFalse(thread.is_alive())
            self.assertEqual(failures, [])


if __name__ == "__main__":
    unittest.main()
