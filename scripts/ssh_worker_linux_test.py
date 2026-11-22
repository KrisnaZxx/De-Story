"""Real Linux FFmpeg checks, using an isolated HOME and synthetic video only."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import unittest
import uuid


@unittest.skipUnless(sys.platform.startswith("linux") and shutil.which("ffmpeg"), "Requires Linux FFmpeg")
class DetachedRenderTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="bara-render-check-")
        self.env = dict(os.environ, HOME=self.tmp.name)
        sibling = Path(__file__).with_name("ssh_worker.py")
        self.source = sibling if sibling.exists() else Path(__file__).resolve().parents[1] / "src-tauri/src/media/ssh_worker.py"
        self.jobs = []

    def tearDown(self):
        for job in self.jobs:
            self.command(job, "cancel", check=False)
        self.tmp.cleanup()

    def create_job(self, seconds=3, failing=False, stages=1):
        job = Path(self.tmp.name) / ".cache/de-story-render" / str(uuid.uuid4())
        job.mkdir(parents=True)
        shutil.copyfile(self.source, job / "worker.py")
        shutil.copyfile(self.source.with_name("ssh_transfer.py"), job / "ssh_transfer.py")
        args = ["-re", "-f", "lavfi", "-i", "color=c=red:s=160x90:r=10", "-t", str(seconds), "-c:v", "libx264", "-threads", "1", "result.mp4"]
        if failing:
            args = ["-invalid-bara-fixture-option", "result.mp4"]
        plan = {"title": "Synthetic render check", "filename": "fixture.mp4", "commands": [{"args": args, "duration": seconds} for _ in range(stages)]}
        (job / "manifest.json").write_text(json.dumps(plan))
        self.jobs.append(job)
        return job

    def command(self, job, action, check=True):
        result = subprocess.run([sys.executable, str(job / "worker.py"), action, job.name],
            cwd=job, env=self.env, capture_output=True, text=True, timeout=15)
        if check:
            self.assertEqual(result.returncode, 0, result.stderr)
            return json.loads(result.stdout)

    def until(self, job, states, timeout=15):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            meta = self.command(job, "status")
            if meta["state"] in states:
                return meta
            time.sleep(.2)
        self.fail(f"Render did not reach {states}: {meta}")

    def test_launcher_exits_and_reconnection_does_not_restart_render(self):
        job = self.create_job()
        self.command(job, "start")  # SSH launcher can exit immediately.
        running = self.until(job, {"running", "completed"})
        duplicate = self.command(job, "start")
        self.assertEqual(duplicate["workerPid"], running["workerPid"])
        result = self.until(job, {"completed", "failed"})
        self.assertEqual(result["state"], "completed", result)
        self.assertEqual(result["sha256"], hashlib.sha256((job / "result.mp4").read_bytes()).hexdigest())
        self.assertEqual(self.command(job, "start")["workerPid"], running["workerPid"])

    def test_cancel_stops_the_current_stage_and_does_not_start_the_next(self):
        job = self.create_job(seconds=10, stages=2)
        self.command(job, "start")
        self.until(job, {"running"})
        self.command(job, "cancel", check=False)
        result = self.until(job, {"failed"})
        self.assertEqual(result["error"], "Dibatalkan")
        self.assertFalse((job / "result.mp4").exists())

    def test_failure_is_persisted_and_never_restarted(self):
        job = self.create_job(failing=True)
        self.command(job, "start")
        result = self.until(job, {"failed"})
        self.assertIn("FFmpeg", result["error"])
        self.assertEqual(self.command(job, "start")["state"], "failed")
        self.assertFalse((job / "result.mp4").exists())

    def segment_job(self, seconds=.2, missing=False):
        job = self.create_job(seconds=seconds, stages=2)
        plan = json.loads((job / "manifest.json").read_text())
        plan["segments"] = ["seg_005.mp4", "seg_001.mp4"]
        for item, name in zip(plan["commands"], plan["segments"]):
            item["args"][-1] = name
        if missing:
            plan["segments"][1] = "seg_002.mp4"
        (job / "manifest.json").write_text(json.dumps(plan))
        return job

    def test_segment_batch_keeps_names_hashes_and_can_be_deleted(self):
        job = self.segment_job()
        self.command(job, "start")
        meta = self.until(job, {"completed", "failed"})
        self.assertEqual(meta["state"], "completed", meta)
        self.assertEqual(meta["kind"], "segments")
        self.assertEqual([item["name"] for item in meta["files"]], ["seg_005.mp4", "seg_001.mp4"])
        for item in meta["files"]:
            data = (job / "segments" / item["name"]).read_bytes()
            self.assertEqual(item["sha256"], hashlib.sha256(data).hexdigest())
            self.assertEqual(item["bytes"], len(data))
        self.assertEqual(self.command(job, "get")["files"], meta["files"])
        self.assertFalse((job / "manifest.json").exists())
        self.command(job, "delete")
        self.jobs.remove(job)
        self.assertFalse(job.exists())

    def test_incomplete_segment_batch_is_not_available_for_download(self):
        job = self.segment_job(missing=True)
        self.command(job, "start")
        meta = self.until(job, {"failed"})
        self.assertIn("missing or invalid", meta["error"])
        self.assertFalse((job / "segments").exists())

    def test_cancelled_segment_batch_removes_partial_clips(self):
        job = self.segment_job(seconds=10)
        self.command(job, "start")
        self.until(job, {"running"})
        self.command(job, "cancel", check=False)
        self.until(job, {"failed"})
        self.assertFalse((job / "segments").exists())
        self.assertFalse((job / "seg_005.mp4").exists())

    def test_killed_encoder_records_signal_and_preserves_diagnostics(self):
        job = self.create_job(seconds=10)
        self.command(job, "start")
        deadline = time.monotonic() + 5
        record = job / "ffmpeg.pid.json"
        while not record.exists() and time.monotonic() < deadline:
            time.sleep(.02)
        pid = json.loads(record.read_text())["pid"]
        process = Path("/proc") / str(pid)
        self.assertEqual((process / "cwd").resolve(), job)
        self.assertEqual(Path((process / "cmdline").read_bytes().split(b"\0")[0].decode()).name, "ffmpeg")
        os.kill(pid, signal.SIGKILL)
        meta = self.until(job, {"failed"})
        self.assertEqual(meta["exitCode"], -9)
        self.assertEqual(meta["failedStage"], 1)
        self.assertIn("SIGKILL", meta["error"])
        self.assertIn("penggabungan akhir", meta["error"])
        self.assertNotIn("progress=continue", meta["error"])


if __name__ == "__main__":
    unittest.main()
