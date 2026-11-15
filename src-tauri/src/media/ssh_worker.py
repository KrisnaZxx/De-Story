"""Private SSH renderer and export-result management. No AI credentials are accepted."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import time
import uuid


def root_directory():
    return Path.home() / ".cache" / "de-story-render"


def job_directory(value):
    identifier = str(uuid.UUID(value))
    if identifier != value:
        raise ValueError("Invalid export identifier")
    root = root_directory().resolve()
    directory = root / identifier
    if directory.is_symlink() or directory.resolve().parent != root:
        raise ValueError("Refusing an invalid export directory")
    return directory


def write_meta(directory, value):
    temporary = directory / "meta.json.tmp"
    temporary.write_text(json.dumps(value), encoding="utf-8")
    temporary.replace(directory / "meta.json")


def process_identity(pid):
    proc = Path("/proc") / str(pid)
    return {"start": (proc / "stat").read_text().split()[21],
            "cwd": str((proc / "cwd").resolve()),
            "program": (proc / "cmdline").read_bytes().split(b"\0")[0].decode()}


def running(directory, saved):
    try:
        current = process_identity(int(saved["pid"]))
        return current["start"] == saved["start"] and current["cwd"] == str(directory) and Path(current["program"]).name == "ffmpeg"
    except (FileNotFoundError, ProcessLookupError):
        return False


def stop(directory):
    record = directory / "ffmpeg.pid.json"
    if not record.exists():
        return
    saved = json.loads(record.read_text())
    if running(directory, saved):
        os.kill(int(saved["pid"]), signal.SIGTERM)
        for _ in range(50):
            if not running(directory, saved):
                break
            time.sleep(0.1)
        if running(directory, saved):
            os.kill(int(saved["pid"]), signal.SIGKILL)
    record.unlink(missing_ok=True)


def clean_temporary(directory):
    # Only this application's UUID job directory can reach this operation.
    for path in directory.iterdir():
        if path.name not in ("meta.json", "result.mp4"):
            if path.is_dir() and not path.is_symlink():
                shutil.rmtree(path)
            else:
                path.unlink(missing_ok=True)


def digest(path):
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def storage():
    usage = shutil.disk_usage(Path.home())
    return {"freeGb": round(usage.free / 2**30, 2), "totalGb": round(usage.total / 2**30, 2),
            "usedGb": round(usage.used / 2**30, 2)}


def read_meta(directory):
    if not (directory / "meta.json").exists():
        return {"id": directory.name, "title": "Unggahan belum selesai", "filename": "result.mp4", "createdAt": directory.stat().st_mtime, "state": "incomplete", "bytes": 0}
    value = json.loads((directory / "meta.json").read_text(encoding="utf-8"))
    value["id"] = directory.name
    if value.get("state") == "running":
        alive = job_running(directory, value)
        if not alive and time.time() - value.get("createdAt", 0) > 60:
            value["state"] = "failed"
    return value


def job_running(directory, meta=None):
    record = directory / "ffmpeg.pid.json"
    if record.exists() and running(directory, json.loads(record.read_text())):
        return True
    try:
        meta = meta or json.loads((directory / "meta.json").read_text())
        current = process_identity(int(meta["workerPid"]))
        return current["start"] == meta["workerStart"] and current["cwd"] == str(directory) and Path(current["program"]).name.startswith("python")
    except (KeyError, FileNotFoundError, ProcessLookupError):
        return False


def run_plan():
    directory = job_directory(Path.cwd().name)
    if Path.cwd().resolve() != directory:
        raise ValueError("Wrong render working directory")
    os.chmod(directory, 0o700)
    plan = json.loads((directory / "manifest.json").read_text(encoding="utf-8"))
    commands = plan["commands"]
    meta = {"id": directory.name, "title": plan["title"], "filename": plan["filename"],
            "createdAt": time.time(), "state": "running", "bytes": 0, "sha256": "",
            "workerPid": os.getpid(), "workerStart": process_identity(os.getpid())["start"]}
    write_meta(directory, meta)
    try:
        for index, item in enumerate(commands):
            final = index == len(commands) - 1
            process = subprocess.Popen(["ffmpeg", "-hide_banner", "-y", "-filter_threads", "2",
                "-filter_complex_threads", "2", "-progress", "pipe:2", "-nostats", *item["args"]],
                stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
            record = directory / "ffmpeg.pid.json"
            record.write_text(json.dumps({"pid": process.pid, "start": process_identity(process.pid)["start"]}))
            try:
                for line in process.stderr:
                    if line.startswith("out_time_us="):
                        try:
                            elapsed = int(line.split("=", 1)[1]) / 1_000_000
                            part = min(1, elapsed / max(0.001, item["duration"]))
                            fraction = 0.6 + 0.4 * part if final else 0.6 * (index + part) / max(1, len(commands) - 1)
                            print(f"de_story_fraction={fraction:.6f}", file=sys.stderr, flush=True)
                        except ValueError:
                            pass
                    else:
                        print(line, end="", file=sys.stderr, flush=True)
                if process.wait() != 0:
                    raise RuntimeError(f"FFmpeg failed on render stage {index + 1}")
            finally:
                if process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()
                record.unlink(missing_ok=True)
        output = directory / "result.mp4"
        meta.update(state="completed", bytes=output.stat().st_size, sha256=digest(output))
        write_meta(directory, meta)
        print("de_story_fraction=1", file=sys.stderr, flush=True)
    except BaseException as error:
        meta.update(state="failed", error=str(error))
        write_meta(directory, meta)
        (directory / "result.mp4").unlink(missing_ok=True)
        raise
    finally:
        clean_temporary(directory)


def main():
    mode = sys.argv[1]
    if mode == "run-plan":
        run_plan()
        return
    if mode == "list":
        results = []
        if root_directory().exists():
            for path in root_directory().iterdir():
                try:
                    results.append(read_meta(job_directory(path.name)))
                except (ValueError, OSError):
                    continue
        print(json.dumps({"storage": storage(), "results": sorted(results, key=lambda r: r.get("createdAt", 0), reverse=True)}))
        return
    directory = job_directory(sys.argv[2])
    if mode == "get":
        meta = read_meta(directory)
        if meta.get("state") != "completed" or not (directory / "result.mp4").is_file():
            raise ValueError("Video is not ready for download")
        print(json.dumps(meta))
    elif mode == "delete":
        if job_running(directory):
            raise ValueError("Export is still running; cancel it in De Story first")
        shutil.rmtree(directory)
        print(json.dumps({"storage": storage()}))
    elif mode == "cancel":
        stop(directory)
    else:
        raise ValueError("Unknown worker action")


if __name__ == "__main__":
    def interrupted(number, _frame):
        raise SystemExit(128 + number)
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGHUP, interrupted)
    main()
