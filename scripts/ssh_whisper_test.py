"""Safety checks for the isolated VPS audio-job directory and cancellation."""
import importlib.util
import json
import os
from pathlib import Path
import sys
import shutil
import subprocess
import tempfile
import time
import types
import unittest
from unittest.mock import patch
import uuid

if os.name == "nt":
    sys.modules["fcntl"] = types.SimpleNamespace()
worker_path = Path(os.environ.get("DE_STORY_WHISPER_WORKER", Path(__file__).resolve().parents[1] / "src-tauri/src/speech/ssh_whisper.py"))
spec = importlib.util.spec_from_file_location("ssh_whisper", worker_path)
worker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(worker)


class AudioJobs(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.override = patch.object(worker, "CACHE", self.root)
        self.override.start()
        self.addCleanup(self.override.stop)

    def test_rejects_path_traversal_and_noncanonical_uuid(self):
        for value in ("../audio", "", "/tmp/audio", "A" * 32):
            with self.assertRaises(ValueError):
                worker.job_directory(value)

    def test_clean_only_removes_its_own_audio_job(self):
        current = worker.job_directory(str(uuid.uuid4()))
        other = worker.job_directory(str(uuid.uuid4()))
        current.mkdir()
        other.mkdir()
        (current / "audio.input").write_bytes(b"private audio")
        worker.cancel_and_clean(current)
        self.assertFalse(current.exists())
        self.assertTrue(other.exists())

    def test_stale_process_record_cannot_kill_an_unrelated_process(self):
        directory = worker.job_directory(str(uuid.uuid4()))
        directory.mkdir()
        record = {"pid": 10, "start": "old", "cwd": str(directory), "program": "ffmpeg"}
        (directory / "child.json").write_text(json.dumps(record))
        with patch.object(worker, "identity", return_value={**record, "start": "new"}), patch.object(worker.os, "kill") as kill:
            worker.cancel_and_clean(directory)
            kill.assert_not_called()

    def test_process_must_belong_to_this_job_and_supported_program(self):
        directory = worker.job_directory(str(uuid.uuid4()))
        for cwd, program in ((str(self.root), "ffmpeg"), (str(directory), "other-app")):
            saved = {"pid": 10, "start": "same", "cwd": cwd, "program": program}
            with patch.object(worker, "identity", return_value=saved):
                self.assertFalse(worker.alive(directory, saved))

    @unittest.skipUnless(os.environ.get("DE_STORY_LIVE_WHISPER") == "1", "requires the VPS test model")
    def test_live_cancel_stops_whisper_and_cleans_private_audio(self):
        with patch.object(worker, "CACHE", Path.home() / ".cache/de-story-whisper"):
            directory = worker.job_directory(str(uuid.uuid4()))
        directory.mkdir(mode=0o700)
        shutil.copyfile(worker.ROOT / "source/samples/jfk.wav", directory / "audio.input")
        (directory / "params.json").write_text(json.dumps({"language": "en", "prompt": "And so my fellow Americans"}))
        process = subprocess.Popen([sys.executable, str(worker_path), "run"], cwd=directory, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            saved = None
            deadline = time.monotonic() + 30
            while time.monotonic() < deadline and process.poll() is None:
                record = directory / "child.json"
                if record.exists():
                    saved = json.loads(record.read_text())
                    if saved["program"] == "whisper-cli":
                        break
                time.sleep(0.01)
            self.assertIsNotNone(saved)
            self.assertEqual(saved["program"], "whisper-cli")
            worker.cancel_and_clean(directory)
            process.wait(timeout=10)
            self.assertFalse(worker.alive(directory, saved))
            self.assertFalse(directory.exists())
        finally:
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=10)
            worker.cancel_and_clean(directory)


if __name__ == "__main__":
    unittest.main()
