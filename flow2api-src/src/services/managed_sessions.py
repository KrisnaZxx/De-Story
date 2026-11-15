"""Account-owned Chrome profiles for login and cookie renewal; no Google passwords."""
import asyncio
import json
import os
import shutil
import subprocess
import sys
import time
import uuid
from pathlib import Path

from ..core import paths


class LoginRequired(RuntimeError):
    pass


def checked_id(value):
    if str(uuid.UUID(str(value))) != str(value):
        raise ValueError("ID profil tidak valid")
    return str(value)


def same_account(expected, observed):
    if not observed or (expected and expected.strip().casefold() != observed.strip().casefold()):
        raise LoginRequired("Login dengan akun Google yang sesuai profil ini.")


def chrome_path():
    candidates = []
    if os.name == "nt":
        for env in ("PROGRAMFILES", "PROGRAMFILES(X86)", "LOCALAPPDATA"):
            if os.environ.get(env):
                candidates.append(Path(os.environ[env]) / "Google/Chrome/Application/chrome.exe")
    elif sys.platform == "darwin":
        candidates.append(Path("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"))
    for name in ("google-chrome", "google-chrome-stable", "chromium", "chromium-browser"):
        found = shutil.which(name)
        if found:
            candidates.append(Path(found))
    if os.environ.get("BROWSER_EXECUTABLE_PATH"):
        candidates.append(Path(os.environ["BROWSER_EXECUTABLE_PATH"]))
    return next((str(p) for p in candidates if p.is_file()), None)


class ManagedSessions:
    def __init__(self, root=None):
        self.root = Path(root or paths.data_store_dir() / "flow-login-profiles")
        self.records = None
        self.running = {}
        self.busy = set()
        self.tasks = set()
        self.bulk_running = False
        self.driver = None
        self.browser_gate = asyncio.Semaphore(1)

    def _load(self):
        if self.records is not None:
            return
        self.root.mkdir(parents=True, exist_ok=True, mode=0o700)
        try:
            values = json.loads((self.root / "profiles.json").read_text(encoding="utf-8"))
        except FileNotFoundError:
            values = []
        self.records = {}
        for value in values:
            checked_id(value["id"])
            self.records[value["id"]] = value

    def _save(self):
        self.root.mkdir(parents=True, exist_ok=True, mode=0o700)
        temp = self.root / "profiles.new"
        temp.write_text(json.dumps(list(self.records.values()), ensure_ascii=False), encoding="utf-8")
        os.chmod(temp, 0o600)
        os.replace(temp, self.root / "profiles.json")

    def profile(self, profile_id):
        self._load()
        checked_id(profile_id)
        if profile_id not in self.records:
            raise ValueError("Profil tidak ditemukan")
        return self.records[profile_id]

    def bound(self, token_id):
        self._load()
        return next((p for p in self.records.values() if p.get("tokenId") == token_id), None)

    def attach(self, token=None):
        self._load()
        if token and self.bound(token.id):
            return self.bound(token.id)
        profile = {"id": str(uuid.uuid4()), "tokenId": token.id if token else None,
                   "email": (token.login_account or token.email or "") if token else "",
                   "state": "idle", "lastSync": None, "error": "", "seeded": False}
        self.records[profile["id"]] = profile
        self._save()
        return profile

    def changed(self, profile, state, error=""):
        profile["state"] = state
        profile["error"] = error
        if state == "ready":
            profile["lastSync"] = int(time.time() * 1000)
        self._save()

    async def snapshot(self, manager):
        self._load()
        tokens = await manager.db.get_all_tokens()
        by_id = {t.id: t for t in tokens}
        profiles = []
        for p in self.records.values():
            t = by_id.get(p.get("tokenId"))
            # Only expose account/status metadata, never the cookie jar or CDP endpoint.
            profiles.append({k: p.get(k) for k in ("id", "tokenId", "email", "state", "lastSync", "error")}
                            | {"browserOpen": p["id"] in self.running,
                               "busy": p["id"] in self.busy,
                               "active": t.is_active if t else False})
        config = await manager.db.get_token_refresh_config()
        accounts = [{"tokenId": t.id, "email": t.login_account or t.email,
                     "active": t.is_active, "managed": bool(self.bound(t.id)),
                     "lastSync": t.last_st_refresh_at.isoformat() if t.last_st_refresh_at else None}
                    for t in tokens]
        return {"profiles": profiles, "accounts": accounts, "bulkRunning": self.bulk_running,
                "autoRefresh": config.enabled, "intervalMinutes": config.refresh_interval_minutes}

    async def _driver(self):
        if self.driver is None:
            from playwright.async_api import async_playwright
            self.driver = await async_playwright().start()
        return self.driver

    async def _browser(self, p, manager, visible):
        profile_id = p["id"]
        if profile_id in self.running:
            return self.running[profile_id]["context"]
        executable = chrome_path()
        if not executable:
            raise RuntimeError("Chrome/Chromium belum tersedia. Pasang Chrome atau siapkan runtime Flow2API.")
        directory = self.root / checked_id(profile_id)
        directory.mkdir(mode=0o700, exist_ok=True)
        port_file = directory / "DevToolsActivePort"
        driver = await self._driver()
        browser = None
        # Recover only this app-owned profile after a backend restart.
        if port_file.exists():
            try:
                port = int(port_file.read_text().splitlines()[0])
                if not 1 <= port <= 65535:
                    raise ValueError("Port profil tidak valid")
                browser = await driver.chromium.connect_over_cdp(f"http://127.0.0.1:{port}", timeout=2000)
            except Exception:
                port_file.unlink(missing_ok=True)
        process = None
        if browser is None:
            args = [executable, f"--user-data-dir={directory}", "--remote-debugging-port=0",
                    "--remote-debugging-address=127.0.0.1", "--no-first-run", "--no-default-browser-check", "--restore-last-session"]
            if not visible:
                args += ["--headless=new"]
            args += ["about:blank"]
            process = subprocess.Popen(args, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                       stderr=subprocess.DEVNULL,
                                       creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
            try:
                for _ in range(100):
                    if port_file.exists():
                        port = int(port_file.read_text().splitlines()[0])
                        if not 1 <= port <= 65535:
                            raise ValueError("Port profil tidak valid")
                        browser = await driver.chromium.connect_over_cdp(f"http://127.0.0.1:{port}", timeout=5000)
                        break
                    if process.poll() is not None:
                        raise RuntimeError("Chrome tidak dapat membuka profil. Tutup proses profil yang tertinggal lalu coba lagi.")
                    await asyncio.sleep(.2)
                if browser is None:
                    raise RuntimeError("Chrome belum merespons. Coba buka profil kembali.")
            except BaseException:
                if process.poll() is None:
                    process.terminate()
                raise
        context = browser.contexts[0]
        self.running[profile_id] = {"browser": browser, "context": context, "process": process}
        if not p.get("seeded") and p.get("tokenId"):
            token = await manager.db.get_token(p["tokenId"])
            if token and token.google_cookies:
                from .browser_cookie_utils import build_browser_cookie_targets
                seed_page = await context.new_page()
                cdp = await context.new_cdp_session(seed_page)
                try:
                    await cdp.send("Network.setCookies", {"cookies": build_browser_cookie_targets(token.google_cookies)})
                finally:
                    await cdp.detach()
                    await seed_page.close()
        p["seeded"] = True
        self._save()
        return context

    async def _read(self, p, manager, visible=False):
        context = await self._browser(p, manager, visible)
        page = await context.new_page()
        try:
            response = await page.goto("https://flow.google.com/projects", wait_until="domcontentloaded", timeout=30000)
            if "accounts.google.com" in page.url:
                raise LoginRequired("Selesaikan login atau verifikasi Google pada profil akun ini.")
            if not response or response.status != 200:
                raise RuntimeError("Google Flow belum merespons normal. Coba refresh sesi kembali nanti.")
            try:
                await page.wait_for_function("(window.WIZ_global_data || {}).oPEP7c", timeout=10000)
            except Exception:
                raise LoginRequired("Belum bisa memastikan akun Flow. Buka profil dan selesaikan login/verifikasi.")
            email = await page.evaluate("(window.WIZ_global_data || {}).oPEP7c || ''")
            same_account(p.get("email"), email)
            cookies = [c for c in await context.cookies() if c["domain"].lstrip(".") == "google.com" or c["domain"].endswith(".google.com")]
            from .browser_cookie_utils import is_partitioned_cookie
            cookies = [c for c in cookies if not is_partitioned_cookie(c)]
            return {"email": email, "google_cookies": json.dumps(cookies, separators=(",", ":"))}
        finally:
            if not visible:
                await page.close()

    async def close(self, profile_id):
        runtime = self.running.pop(profile_id, None)
        if runtime:
            try:
                try:
                    cdp = await runtime["browser"].new_browser_cdp_session()
                    await cdp.send("Browser.close")
                except Exception:
                    pass
                await runtime["browser"].close()
            finally:
                process = runtime.get("process")
                if process and process.poll() is None:
                    try:
                        await asyncio.wait_for(asyncio.to_thread(process.wait), 5)
                    except asyncio.TimeoutError:
                        process.terminate()

    async def open_login(self, manager, profile_id):
        p = self.profile(profile_id)
        if profile_id in self.busy:
            raise RuntimeError("Profil sedang memperbarui sesi; tunggu sebentar.")
        self.busy.add(profile_id)
        try:
            context = await self._browser(p, manager, True)
            page = context.pages[0] if context.pages else await context.new_page()
            await page.goto("https://flow.google.com/projects", wait_until="domcontentloaded", timeout=30000)
            self.changed(p, "login")
            instance = self.running[profile_id]
            async def expire_login():
                await asyncio.sleep(1200)
                if self.running.get(profile_id) is instance and profile_id not in self.busy:
                    await self.close(profile_id)
                    self.changed(p, "idle", "Browser login ditutup setelah 20 menit. Buka profil untuk melanjutkan.")
            self.schedule(expire_login())
        finally:
            self.busy.discard(profile_id)

    async def complete(self, manager, profile_id):
        p = self.profile(profile_id)
        if profile_id in self.busy:
            raise RuntimeError("Profil sedang diproses.")
        self.busy.add(profile_id)
        try:
            result = await self._read(p, manager, True)
            token = await manager.db.get_token(p["tokenId"]) if p.get("tokenId") else await manager.db.get_token_by_email(result["email"])
            if token:
                other = self.bound(token.id)
                if other and other["id"] != profile_id:
                    raise RuntimeError("Akun ini sudah mempunyai profil tersimpan. Gunakan profil akun yang sudah ada.")
                same_account(token.login_account or token.email, result["email"])
                await manager._save_refreshed_cookies(token.id, token, result)
            else:
                refresh = await manager.db.get_token_refresh_config()
                token = await manager.add_token("", google_cookies=result["google_cookies"], login_account=result["email"], refresh_interval_minutes=refresh.refresh_interval_minutes)
            # Avoid binding one token to several independent mutable profiles.
            other = self.bound(token.id)
            if other and other["id"] != profile_id:
                raise RuntimeError("Akun ini sudah mempunyai profil tersimpan. Gunakan profil akun yang sudah ada.")
            p["tokenId"], p["email"] = token.id, result["email"]
            self.changed(p, "ready")
            await self.close(profile_id)
        except LoginRequired as exc:
            self.changed(p, "needs_login", str(exc))
            raise
        finally:
            self.busy.discard(profile_id)

    async def refresh_bound(self, manager, token):
        p = self.bound(token.id)
        if not p:
            return None
        if p["id"] in self.busy or p["id"] in self.running:
            raise RuntimeError("Profil sedang dibuka untuk login. Selesaikan login lalu simpan sesi.")
        self.busy.add(p["id"])
        self.changed(p, "refreshing")
        try:
            async with self.browser_gate:
                result = await self._read(p, manager)
                same_account(token.login_account or token.email, result["email"])
                return result
        except LoginRequired as exc:
            self.changed(p, "needs_login", str(exc))
            raise
        except Exception:
            self.changed(p, "error", "Pembaruan sesi gagal. Periksa koneksi, lalu coba lagi.")
            raise
        finally:
            await self.close(p["id"])
            self.busy.discard(p["id"])

    def mark_refreshed(self, token_id):
        p = self.bound(token_id)
        if p and p["id"] not in self.running:
            self.changed(p, "ready")

    async def renew(self, manager, token_id):
        token = await manager.db.get_token(token_id)
        if not token:
            raise ValueError("Akun Flow tidak ditemukan")
        p = self.bound(token_id)
        if p and (p["id"] in self.running or p["id"] in self.busy):
            return
        success = await manager.refresh_cookies(token_id)
        if p and success:
            self.changed(p, "ready")
        elif p and p["state"] not in ("needs_login", "error"):
            self.changed(p, "error", "Refresh belum berhasil. Buka profil jika Google meminta login ulang.")

    def schedule(self, coroutine):
        async def safe():
            try:
                await coroutine
            except Exception:
                pass  # Account state records errors; never emit cookies in background task errors.
        task = asyncio.create_task(safe())
        self.tasks.add(task)
        task.add_done_callback(self.tasks.discard)

    async def renew_all(self, manager, attach=False):
        if self.bulk_running:
            return
        self.bulk_running = True
        try:
            for token in await manager.db.get_all_tokens():
                if not token.is_active:
                    continue
                if attach:
                    p = self.attach(token)
                    if not p.get("seeded"):
                        try:
                            result = await self.refresh_bound(manager, token)
                            await manager._save_refreshed_cookies(token.id, token, result)
                        except Exception:
                            continue
                try:
                    await self.renew(manager, token.id)
                except Exception:
                    p = self.bound(token.id)
                    if p:
                        self.changed(p, "error", "Pembaruan gagal; akun lain tetap diproses.")
        finally:
            self.bulk_running = False

    async def shutdown(self):
        for task in self.tasks:
            task.cancel()
        await asyncio.gather(*list(self.tasks), return_exceptions=True)
        for profile_id in list(self.running):
            await self.close(profile_id)
        if self.driver:
            await self.driver.stop()


managed_sessions = ManagedSessions()
