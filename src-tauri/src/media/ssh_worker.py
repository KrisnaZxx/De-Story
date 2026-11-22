"""Private SSH renderer and export-result management. No AI credentials are accepted."""
import hashlib
import json
import os
import re
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
        if path.name not in ("meta.json", "result.mp4", "segments", "worker.py", "ssh_transfer.py", "start.lock"):
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
    if value.get("state") in ("queued", "running"):
        alive = job_running(directory, value)
        if not alive and time.time() - value.get("createdAt", 0) > 60:
            value["state"] = "failed"
    return value


def segment_outputs(plan):
    outputs = plan.get("segments", [])
    if (not isinstance(outputs, list) or len(outputs) > 5000
            or any(not isinstance(name, str) or not re.fullmatch(r"seg_[0-9]{3,6}\.mp4", name) for name in outputs)
            or len(set(outputs)) != len(outputs)):
        raise ValueError("Invalid segment output names")
    return outputs


def progress_line(line):
    return re.match(r"^(frame|fps|stream_\d+_\d+_q|bitrate|total_size|out_time(_us|_ms)?|dup_frames|drop_frames|speed|progress)=", line) is not None


def start(directory):
    # The lock and persisted state make an uncertain SSH response safe to retry.
    import fcntl
    with (directory / "start.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        if (directory / "meta.json").exists():
            return read_meta(directory)
        plan = json.loads((directory / "manifest.json").read_text(encoding="utf-8"))
        outputs = segment_outputs(plan)
        write_meta(directory, {"id": directory.name, "title": plan["title"],
            "filename": plan["filename"], "createdAt": time.time(), "state": "queued", "bytes": 0,
            "kind": "segments" if outputs else "video"})
        try:
            subprocess.Popen([sys.executable, str(directory / "worker.py"), "run-plan"],
                cwd=directory, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL, start_new_session=True, close_fds=True)
        except OSError as error:
            meta = read_meta(directory)
            meta.update(state="failed", error=str(error))
            write_meta(directory, meta)
            raise
        return read_meta(directory)


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
    outputs = segment_outputs(plan)
    meta = {"id": directory.name, "title": plan["title"], "filename": plan["filename"],
            "createdAt": time.time(), "state": "running", "bytes": 0, "sha256": "",
            "workerPid": os.getpid(), "workerStart": process_identity(os.getpid())["start"],
            "kind": "segments" if outputs else "video"}
    write_meta(directory, meta)
    try:
        for index, item in enumerate(commands):
            if (directory / "cancelled").exists():
                raise RuntimeError("Dibatalkan")
            final = index == len(commands) - 1
            process = subprocess.Popen(["ffmpeg", "-hide_banner", "-y", "-filter_threads", "2",
                "-filter_complex_threads", "2", "-progress", "pipe:2", "-nostats", *item["args"]],
                stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
            record = directory / "ffmpeg.pid.json"
            record.write_text(json.dumps({"pid": process.pid, "start": process_identity(process.pid)["start"]}))
            tail = []
            try:
                if (directory / "cancelled").exists():
                    raise RuntimeError("Dibatalkan")
                for line in process.stderr:
                    if line.startswith("out_time_us="):
                        try:
                            elapsed = int(line.split("=", 1)[1]) / 1_000_000
                            part = min(1, elapsed / max(0.001, item["duration"]))
                            fraction = ((index + part) / len(commands) if outputs else part if len(commands) == 1
                                        else 0.6 + 0.4 * part if final else 0.6 * (index + part) / (len(commands) - 1))
                            meta["progress"] = max(meta.get("progress", 0), fraction)
                            write_meta(directory, meta)
                        except ValueError:
                            pass
                    elif not progress_line(line):
                        tail = (tail + [line.strip()])[-12:]
                code = process.wait()
                if code != 0:
                    meta.update(exitCode=code, failedStage=index + 1)
                    killed = " SIGKILL: proses dihentikan sistem; periksa RAM atau batas VPS." if code == -9 else ""
                    phase = "penggabungan akhir" if final and not outputs else "render klip"
                    raise RuntimeError(f"FFmpeg gagal saat {phase} (tahap {index + 1}, exit {code}).{killed}\n" + "\n".join(tail))
            finally:
                if process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()
                record.unlink(missing_ok=True)
        if (directory / "cancelled").exists():
            raise RuntimeError("Dibatalkan")
        if outputs:
            segments = directory / "segments"
            segments.mkdir()
            files = []
            for name in outputs:
                output = directory / name
                if output.is_symlink() or not output.is_file():
                    raise ValueError("Segment output missing or invalid")
                output.replace(segments / name)
                files.append({"name": name, "bytes": (segments / name).stat().st_size, "sha256": digest(segments / name)})
            meta.update(state="completed", progress=1, files=files, bytes=sum(item["bytes"] for item in files))
        else:
            output = directory / "result.mp4"
            meta.update(state="completed", progress=1, bytes=output.stat().st_size, sha256=digest(output))
        write_meta(directory, meta)
    except BaseException as error:
        meta.update(state="failed", error="Dibatalkan" if (directory / "cancelled").exists() else str(error))
        write_meta(directory, meta)
        (directory / "result.mp4").unlink(missing_ok=True)
        if (directory / "segments").is_dir():
            shutil.rmtree(directory / "segments")
        raise
    finally:
        clean_temporary(directory)


def main():
    mode = sys.argv[1]
    if mode in ("start-transfer", "serve-transfer", "stop-transfer", "receive-transfer"):
        import ssh_transfer
        directory = job_directory(sys.argv[2])
        if mode == "start-transfer":
            print(json.dumps(ssh_transfer.start(directory)))
        elif mode == "serve-transfer":
            ssh_transfer.serve(directory)
        elif mode == "stop-transfer":
            ssh_transfer.stop(directory)
        else:
            name = sys.argv[3]
            if not re.fullmatch(r"transfer-[a-f0-9-]{36}\.json", name):
                raise ValueError("Invalid transfer ticket")
            ticket = directory / name
            try:
                print(json.dumps(ssh_transfer.receive(directory, json.loads(ticket.read_text()))))
            finally:
                ticket.unlink(missing_ok=True)
        return
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
    if mode == "start":
        print(json.dumps(start(directory)))
    elif mode == "status":
        print(json.dumps(read_meta(directory)))
    elif mode == "get":
        meta = read_meta(directory)
        output = directory / ("segments" if meta.get("kind") == "segments" else "result.mp4")
        if meta.get("state") != "completed" or not output.exists():
            raise ValueError("Video is not ready for download")
        print(json.dumps(meta))
    elif mode == "delete":
        if job_running(directory):
            raise ValueError("Export is still running; cancel it in De Story first")
        if (directory / "transfer.pid.json").exists():
            import ssh_transfer
            ssh_transfer.stop(directory)
        shutil.rmtree(directory)
        print(json.dumps({"storage": storage()}))
    elif mode == "cancel":
        (directory / "cancelled").touch()
        stop(directory)
        if (directory / "transfer.pid.json").exists():
            import ssh_transfer
            ssh_transfer.stop(directory)
    else:
        raise ValueError("Unknown worker action")


if __name__ == "__main__":
    sys.modules.setdefault("ssh_worker", sys.modules[__name__])
    def interrupted(number, _frame):
        raise SystemExit(128 + number)
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGHUP, interrupted)
    main()
