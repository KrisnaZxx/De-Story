"""Check deletion boundaries and preservation of completed remote exports."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import uuid

source = Path(__file__).resolve().parents[1] / "src-tauri/src/media/ssh_worker.py"
spec = importlib.util.spec_from_file_location("ssh_worker", source)
worker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(worker)


class WorkerSafetyTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name) / "jobs"
        self.root.mkdir()
        self.identifier = str(uuid.uuid4())
        self.directory = self.root / self.identifier
        self.directory.mkdir()
        self.root_patch = patch.object(worker, "root_directory", return_value=self.root)
        self.root_patch.start()

    def tearDown(self):
        self.root_patch.stop()
        self.temporary.cleanup()

    def test_job_directory_rejects_traversal(self):
        for invalid in ("../outside", "", self.identifier.upper(), self.identifier + "/.."): 
            with self.assertRaises(ValueError):
                worker.job_directory(invalid)
        self.assertEqual(worker.job_directory(self.identifier), self.directory.resolve())

    def test_cleanup_preserves_result_metadata_and_other_jobs(self):
        (self.directory / "meta.json").write_text('{}')
        (self.directory / "result.mp4").write_bytes(b"completed video")
        (self.directory / "seg_000.mp4").write_bytes(b"temporary segment")
        (self.directory / "inputs").mkdir()
        (self.directory / "inputs/source.png").write_bytes(b"source")
        other = self.root / str(uuid.uuid4())
        other.mkdir()
        (other / "result.mp4").write_bytes(b"other video")
        worker.clean_temporary(self.directory)
        self.assertEqual(sorted(p.name for p in self.directory.iterdir()), ["meta.json", "result.mp4"])
        self.assertEqual((other / "result.mp4").read_bytes(), b"other video")

    def test_stale_pid_cannot_kill_a_different_process(self):
        (self.directory / "ffmpeg.pid.json").write_text(json.dumps({"pid": 123, "start": "old"}))
        with patch.object(worker, "process_identity", return_value={"start": "new", "cwd": str(self.directory), "program": "ffmpeg"}), patch.object(worker.os, "kill") as kill:
            worker.stop(self.directory)
            kill.assert_not_called()

    def test_active_export_cannot_be_deleted(self):
        with patch.object(worker.sys, "argv", ["worker", "delete", self.identifier]), patch.object(worker, "job_running", return_value=True):
            with self.assertRaisesRegex(ValueError, "still running"):
                worker.main()
        self.assertTrue(self.directory.exists())

    def test_incomplete_upload_is_visible_for_storage_cleanup(self):
        meta = worker.read_meta(self.directory)
        self.assertEqual(meta["id"], self.identifier)
        self.assertEqual(meta["state"], "incomplete")


if __name__ == "__main__":
    unittest.main()
