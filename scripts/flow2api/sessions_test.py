"""Offline checks: no Google requests, real accounts, or user's data directory."""
import asyncio
import json
import logging
import os
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import AsyncMock

scratch = tempfile.TemporaryDirectory(prefix="de-story-session-check-")
os.environ["FLOW2API_DATA_DIR"] = scratch.name
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "flow2api-src"))
from src.services.managed_sessions import ManagedSessions, LoginRequired, checked_id, same_account


def account(number):
    return SimpleNamespace(id=number, login_account=f"demo{number}@example.test", email=f"demo{number}@example.test",
                           is_active=True, google_cookies="private-cookie-must-not-appear", last_st_refresh_at=None)


class SessionsTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(dir=scratch.name)
        self.sessions = ManagedSessions(Path(self.directory.name))
        self.tokens = [account(1), account(2)]
        self.manager = SimpleNamespace(db=SimpleNamespace(
            get_all_tokens=AsyncMock(return_value=self.tokens),
            get_token=AsyncMock(side_effect=lambda token_id: next(t for t in self.tokens if t.id == token_id)),
            get_token_refresh_config=AsyncMock(return_value=SimpleNamespace(enabled=True, refresh_interval_minutes=30))),
            refresh_cookies=AsyncMock(return_value=True), _save_refreshed_cookies=AsyncMock(), enable_token=AsyncMock())

    async def asyncTearDown(self):
        await self.sessions.shutdown()
        self.directory.cleanup()

    async def test_account_binding_persistence_and_public_snapshot(self):
        p = self.sessions.attach(self.tokens[0])
        self.assertEqual(p["id"], self.sessions.attach(self.tokens[0])["id"])
        reloaded = ManagedSessions(Path(self.directory.name))
        self.assertEqual(reloaded.bound(1)["email"], self.tokens[0].email)
        result = await self.sessions.snapshot(self.manager)
        self.assertNotIn("private-cookie", json.dumps(result))
        self.assertNotIn("DevTools", json.dumps(result))
        for value in ["../other", "1", "", "/tmp/not-a-profile"]:
            with self.assertRaises(ValueError):
                checked_id(value)
        same_account("DEMO@example.test", "demo@example.test")
        with self.assertRaises(LoginRequired):
            same_account("first@example.test", "other@example.test")

    async def test_healthy_renewal_does_not_open_browser(self):
        p = self.sessions.attach(self.tokens[0])
        self.sessions._browser = AsyncMock(side_effect=AssertionError("Browser must remain closed"))
        await self.sessions.renew(self.manager, 1)
        self.assertEqual(p["state"], "ready")
        self.assertIsNotNone(p["lastSync"])
        self.sessions._browser.assert_not_called()

    async def test_manually_closed_login_browser_is_released(self):
        p = self.sessions.attach(self.tokens[0])
        self.sessions.changed(p, "login")
        browser = SimpleNamespace(is_connected=lambda: False, close=AsyncMock())
        self.sessions.running[p["id"]] = {"browser":browser,"process":None}
        result = await self.sessions.snapshot(self.manager)
        self.assertFalse(result["profiles"][0]["browserOpen"])
        self.assertEqual(result["profiles"][0]["state"], "idle")
        self.assertIsNotNone(self.sessions.bound(1))
        browser.close.assert_awaited_once()

    async def test_open_login_is_not_disturbed_by_background_renewal(self):
        p = self.sessions.attach(self.tokens[0])
        self.sessions.busy.add(p["id"])
        await self.sessions.renew(self.manager, 1)
        self.manager.refresh_cookies.assert_not_called()
        with self.assertRaises(RuntimeError):
            await self.sessions.refresh_bound(self.manager, self.tokens[0])
        self.sessions.busy.clear()

    async def test_wrong_account_cannot_replace_existing_session(self):
        p = self.sessions.attach(self.tokens[0])
        self.sessions._read = AsyncMock(return_value={"email":"other@example.test", "google_cookies":"not-a-real-cookie"})
        with self.assertRaises(LoginRequired):
            await self.sessions.complete(self.manager, p["id"])
        self.manager._save_refreshed_cookies.assert_not_called()
        self.manager.enable_token.assert_not_called()
        self.assertNotEqual(p["state"], "ready")

    async def test_expired_account_is_reenabled_only_after_successful_login(self):
        token = self.tokens[0]
        token.is_active = False
        p = self.sessions.attach(token)
        self.sessions._read = AsyncMock(return_value={"email":token.email,"google_cookies":"validated-test-cookie"})
        self.sessions.close = AsyncMock()
        await self.sessions.complete(self.manager,p["id"])
        self.manager._save_refreshed_cookies.assert_awaited_once()
        self.manager.enable_token.assert_awaited_once_with(token.id)
        self.assertEqual(p["state"], "ready")
        self.manager.enable_token.reset_mock()
        self.manager._save_refreshed_cookies.side_effect = RuntimeError("Invalid refreshed credentials")
        with self.assertRaises(RuntimeError):
            await self.sessions.complete(self.manager,p["id"])
        self.manager.enable_token.assert_not_called()

    async def test_bulk_keeps_processing_other_accounts_and_releases_busy_flag(self):
        self.sessions.renew = AsyncMock(side_effect=[RuntimeError("offline"), None])
        self.sessions.attach(self.tokens[0])
        self.sessions.attach(self.tokens[1])
        await self.sessions.renew_all(self.manager)
        self.assertEqual(self.sessions.renew.await_count, 2)
        self.assertFalse(self.sessions.bulk_running)

    async def test_existing_cookie_profiles_are_seeded_before_normal_refresh(self):
        async def seeded(manager, token):
            self.sessions.bound(token.id)["seeded"] = True
            return {"email":token.email,"google_cookies":"fresh-test-cookie"}
        self.sessions.refresh_bound = AsyncMock(side_effect=seeded)
        await self.sessions.renew_all(self.manager, attach=True)
        self.assertEqual(self.sessions.refresh_bound.await_count, 2)
        self.assertEqual(self.manager._save_refreshed_cookies.await_count, 2)
        self.assertTrue(all(p["seeded"] for p in self.sessions.records.values()))

    async def test_admin_session_endpoints_require_authentication(self):
        from fastapi import FastAPI
        import httpx
        from src.api.studio_sessions import router
        app = FastAPI()
        app.include_router(router)
        async with httpx.AsyncClient(transport=httpx.ASGITransport(app=app), base_url="http://test") as client:
            self.assertEqual((await client.get("/api/studio/sessions")).status_code, 401)
            self.assertEqual((await client.post("/api/studio/sessions", json={"action":"add"})).status_code, 401)


if __name__ == "__main__":
    try:
        unittest.main(verbosity=2)
    finally:
        logging.shutdown()
        assert Path(scratch.name).resolve().parent == Path(tempfile.gettempdir()).resolve()
        scratch.cleanup()
