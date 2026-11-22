"""Job-scoped direct transfers through InstaBox private HTTPS ports."""
import hashlib
import http.client
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import time
from urllib.parse import urlsplit


def files(meta):
    if not isinstance(meta, dict):
        raise ValueError("Invalid transfer metadata")
    items = meta.get("files", [])
    if not items or not isinstance(items, list):
        raise ValueError("Missing transfer files")
    seen = set()
    for item in items:
        if not isinstance(item, dict):
            raise ValueError("Invalid transfer metadata")
        name = item.get("name", "")
        checksum = item.get("sha256", "")
        if (not isinstance(name, str) or not re.fullmatch(r"seg_[0-9]{3,6}\.mp4", name) or name in seen
                or not isinstance(checksum, str) or not re.fullmatch(r"[a-f0-9]{64}", checksum)
                or not isinstance(item.get("bytes"), int) or item["bytes"] <= 0):
            raise ValueError("Invalid transfer metadata")
        seen.add(name)
    return items


def stop(directory):
    from ssh_worker import process_identity
    record = directory / "transfer.pid.json"
    if not record.exists():
        return
    saved = json.loads(record.read_text())
    try:
        current = process_identity(saved["pid"])
        if current["cwd"] == str(directory) and current["start"] == saved["start"] and Path(current["program"]).name.startswith("python"):
            os.kill(saved["pid"], signal.SIGTERM)
            for _ in range(100):
                if not record.exists():
                    break
                time.sleep(.1)
    except (ProcessLookupError, FileNotFoundError):
        pass
    if record.exists() and saved.get("port"):
        result = subprocess.run(["instabox-port", "unpublish", str(saved["port"])], capture_output=True, timeout=20)
        if result.returncode not in (0, 4):
            raise RuntimeError("Port transfer belum dapat ditutup; periksa Ports di InstaBox")
    record.unlink(missing_ok=True)


def serve(directory):
    from ssh_worker import process_identity
    os.chmod(directory, 0o700)
    pid = directory / "transfer.pid.json"
    pid.write_text(json.dumps({"pid": os.getpid(), "start": process_identity(os.getpid())["start"]}))
    port = None
    try:
        meta = json.loads((directory / "meta.json").read_text())
        if meta.get("state") != "completed" or meta.get("kind") != "segments":
            raise ValueError("Segments are not ready")
        outputs = {item["name"]: item for item in files(meta)}

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_GET(self):
                name = self.path.removeprefix("/")
                path = directory / "segments" / name
                if name not in outputs or path.is_symlink() or not path.is_file():
                    self.send_error(404)
                    return
                with path.open("rb") as stream:
                    self.send_response(200)
                    self.send_header("Content-Length", str(outputs[name]["bytes"]))
                    self.send_header("Content-Type", "video/mp4")
                    self.end_headers()
                    shutil.copyfileobj(stream, self.wfile, 1024 * 1024)

        listing = subprocess.run(["instabox-port", "list"], capture_output=True, text=True, timeout=20)
        if listing.returncode != 0:
            raise RuntimeError("Cannot inspect existing published ports")
        occupied = {item["port"] for item in json.loads(listing.stdout)["ports"]}
        for _ in range(20):
            server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
            if server.server_port not in occupied:
                break
            server.server_close()
        else:
            raise RuntimeError("Cannot allocate a new private transfer port")
        with server:
            port = server.server_port
            pid.write_text(json.dumps({"pid":os.getpid(),"start":process_identity(os.getpid())["start"],"port":port}))
            published = subprocess.run(["instabox-port", "publish", str(port), "--private", "--name", "Bara render " + directory.name, "--json"], capture_output=True, text=True, timeout=30)
            if published.returncode != 0:
                raise RuntimeError("Tidak dapat membuka port HTTPS privat InstaBox; periksa kuota port VPS")
            access = json.loads(published.stdout)
            if access.get("access") != "private" or not access.get("token") or urlsplit(access.get("url", "")).scheme != "https":
                raise ValueError("Private HTTPS port response invalid")
            ready = directory / "transfer.ready.json"
            ready.write_text(json.dumps({"url": access["url"], "token": access["token"], "files": list(outputs.values())}))
            os.chmod(ready, 0o600)
            server.timeout = 1
            deadline = time.monotonic() + 3600
            while time.monotonic() < deadline:
                server.handle_request()
    except Exception:
        (directory / "transfer.error").write_text("Transfer langsung tidak dapat dimulai; periksa port privat InstaBox pada VPS sumber")
        raise
    finally:
        closed = port is None
        try:
            if port is not None:
                result = subprocess.run(["instabox-port", "unpublish", str(port)], capture_output=True, timeout=20)
                closed = result.returncode in (0, 4)
        finally:
            if closed:
                pid.unlink(missing_ok=True)
            (directory / "transfer.ready.json").unlink(missing_ok=True)


def start(directory):
    if not shutil.which("instabox-port"):
        raise ValueError("Transfer langsung memerlukan instabox-port pada VPS sumber; gunakan satu VPS untuk server lain")
    for name in ("transfer.ready.json", "transfer.error"):
        (directory / name).unlink(missing_ok=True)
    subprocess.Popen([sys.executable, str(directory / "worker.py"), "serve-transfer", directory.name], cwd=directory,
        stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
    for _ in range(350):
        if (directory / "transfer.ready.json").exists():
            return json.loads((directory / "transfer.ready.json").read_text())
        if (directory / "transfer.error").exists():
            raise RuntimeError((directory / "transfer.error").read_text())
        time.sleep(.1)
    stop(directory)
    raise TimeoutError("Port transfer langsung tidak siap dalam 35 detik")


def receive(directory, ticket):
    outputs = files(ticket)
    url = urlsplit(ticket.get("url", ""))
    local = ticket.get("localJob")
    if local:
        from ssh_worker import job_directory
        source = job_directory(local) / "segments"
    elif url.scheme != "https" or not url.hostname or url.username or url.password or url.query or url.fragment or url.path not in ("", "/"):
        raise ValueError("Transfer requires a private HTTPS origin")
    for item in outputs:
        target = directory / item["name"]
        partial = directory / (item["name"] + ".part")
        if target.exists():
            raise ValueError("Refusing to overwrite an existing segment")
        created = False
        try:
            digest = hashlib.sha256()
            total = 0
            with partial.open("xb") as out:
                created = True
                if local:
                    path = source / item["name"]
                    if path.is_symlink() or not path.is_file():
                        raise ValueError("Invalid local segment")
                    stream = path.open("rb")
                    connection = None
                else:
                    connection = http.client.HTTPSConnection(url.hostname, url.port or 443, timeout=30)
                    connection.request("GET", "/" + item["name"], headers={"Authorization": "Bearer " + ticket["token"]})
                    stream = connection.getresponse()
                    if stream.status != 200:
                        connection.close()
                        raise ValueError("Direct HTTPS transfer rejected")
                try:
                    while True:
                        block = stream.read(1024 * 1024)
                        if not block:
                            break
                        total += len(block)
                        if total > item["bytes"]:
                            raise ValueError("Segment exceeded expected size")
                        digest.update(block)
                        out.write(block)
                finally:
                    stream.close()
                    if connection:
                        connection.close()
            if total != item["bytes"] or digest.hexdigest() != item["sha256"]:
                raise ValueError("Checksum/size mismatch; source segments retained")
            partial.rename(target)
        finally:
            if created:
                partial.unlink(missing_ok=True)
    return {"files": outputs}
