import tempfile
import unittest
from pathlib import Path

from cleanup import cleanup
from isolate import isolate
from test_ops import git, repository


class CleanupTests(unittest.TestCase):
    def test_verified_toy_work_is_archived_and_a_late_edit_prevents_cleanup(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source, task = root / "source", root / "B123"
            repository(source)
            isolate(source, task, "task/B123-goose")
            (task / "goose.txt").write_text("honk\n")
            git(task, "add", "goose.txt")
            git(task, "commit", "-qm", "teach goose to honk")
            head = git(task, "rev-parse", "HEAD").decode().strip()
            (task / "unfinished.txt").write_text("do not lose this\n")
            with self.assertRaisesRegex(RuntimeError, "changed after"):
                cleanup(source, task, head)
            self.assertTrue((task / "unfinished.txt").exists())
            (task / "unfinished.txt").unlink()
            cleanup(source, task, head)
            self.assertFalse(task.exists())
            self.assertEqual(git(source, "show", "refs/betterlite/completed/B123:goose.txt"), b"honk\n")


if __name__ == "__main__":
    unittest.main()
