"""Check that bundled Flow distributes images without overlapping one account."""
import asyncio
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import AsyncMock, Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "flow2api-src"))
from src.services.concurrency_manager import ConcurrencyManager
from src.services.load_balancer import LoadBalancer
from src.core.models import Token
from src.core.database import Database
from src.services.flow_quota import is_quota_error


class AccountConcurrencyTest(unittest.IsolatedAsyncioTestCase):
    async def test_generation_switches_accounts_only_after_explicit_quota_rejection(self):
        import json
        from src.services.generation_handler import GenerationHandler, MODEL_CONFIG
        for uncertain in (False, True):
            accounts = [Token(id=i, st="test", email=f"account{i}@example.test", credits=10) for i in range(1, 6)]
            tm = Mock(get_active_tokens=AsyncMock(return_value=accounts), ensure_valid_token=AsyncMock(side_effect=lambda t: t),
                      ensure_project_exists=AsyncMock(return_value="demo"), record_error=AsyncMock(), record_usage=AsyncMock(), record_success=AsyncMock())
            tm.needs_at_refresh.return_value = False
            balancer = LoadBalancer(tm)
            balancer._check_extension_route = AsyncMock(return_value=(True, ""))
            handler = GenerationHandler.__new__(GenerationHandler)
            handler.token_manager, handler.load_balancer = tm, balancer
            handler.concurrency_manager = None
            handler.flow_client = Mock(prefill_remote_browser_pool=AsyncMock())
            handler._log_request = AsyncMock(return_value=1)
            handler._update_request_log_progress = AsyncMock()
            called = []
            async def image(token, *args, **kwargs):
                called.append(token.id)
                if len(called) == 1:
                    error = RuntimeError("PUBLIC_ERROR_INSUFFICIENT_CREDITS")
                    error.submission_uncertain = uncertain
                    raise error
                handler._mark_generation_succeeded(kwargs["generation_result"])
                kwargs["response_state"]["url"] = "https://example.test/result.png"
                yield handler._create_completion_response("https://example.test/result.png")
            handler._handle_image_generation = image
            model = next(k for k, v in MODEL_CONFIG.items() if v["type"] == "image")
            with patch("src.services.generation_handler.supports_model_for_tier", return_value=True):
                chunks = [chunk async for chunk in handler.handle_generation(model, "demo")]
            if uncertain:
                self.assertEqual(len(called), 1)
                self.assertIn("error", json.loads(chunks[-1]))
            else:
                self.assertEqual(len(called), 2)
                self.assertNotEqual(called[0], called[1])
                self.assertIn("choices", json.loads(chunks[-1]))
            self.assertEqual(balancer._image_pending, {})

    async def test_quota_failover_waits_for_busy_healthy_account_without_reusing_exhausted_one(self):
        from src.services.generation_handler import GenerationHandler, MODEL_CONFIG
        accounts = [Token(id=i, st="test", email=f"a{i}@example.test", credits=10) for i in range(1, 3)]
        tm = Mock(get_active_tokens=AsyncMock(return_value=accounts), ensure_valid_token=AsyncMock(side_effect=lambda t: t),
                  ensure_project_exists=AsyncMock(return_value="demo"), record_error=AsyncMock(), record_usage=AsyncMock(), record_success=AsyncMock())
        tm.needs_at_refresh.return_value = False
        manager = ConcurrencyManager()
        balancer = LoadBalancer(tm, manager)
        balancer._check_extension_route = AsyncMock(return_value=(True, ""))
        handler = GenerationHandler.__new__(GenerationHandler)
        handler.token_manager, handler.load_balancer, handler.concurrency_manager = tm, balancer, manager
        handler.flow_client = Mock(prefill_remote_browser_pool=AsyncMock())
        handler._log_request = AsyncMock(return_value=1)
        handler._update_request_log_progress = AsyncMock()
        rejected = asyncio.Event()
        called = []
        async def image(token, *args, **kwargs):
            called.append(token.id)
            if token.id == 1:
                rejected.set()
                raise RuntimeError("PUBLIC_ERROR_QUOTA_EXCEEDED")
            handler._mark_generation_succeeded(kwargs["generation_result"])
            yield handler._create_completion_response("https://example.test/result.png")
        async def finish_busy():
            await rejected.wait()
            await asyncio.sleep(.05)
            await manager.release_image(2)
        handler._handle_image_generation = image
        model = next(k for k, v in MODEL_CONFIG.items() if v["type"] == "image")
        with patch("src.services.concurrency_manager.is_embedded", return_value=True), patch("src.services.generation_handler.is_embedded", return_value=True), patch("src.services.generation_handler.supports_model_for_tier", return_value=True), patch("src.services.load_balancer.supports_model_for_tier", return_value=True):
            await manager.acquire_image(2)
            other_job = asyncio.create_task(finish_busy())
            chunks = [chunk async for chunk in handler.handle_generation(model, "demo")]
            await other_job
            self.assertEqual(called, [1, 2])
            self.assertEqual(await manager.get_image_inflight(1), 0)
            self.assertEqual(await manager.get_image_inflight(2), 0)
            self.assertNotIn('"error":', chunks[-1])

    async def test_credit_exhaustion_skips_an_account_for_both_media_and_recovers_after_topup(self):
        accounts = [Token(id=i, st="test", email=f"account{i}@example.test", credits=10) for i in range(1, 6)]
        tm = Mock(get_active_tokens=AsyncMock(return_value=accounts), ensure_valid_token=AsyncMock(side_effect=lambda t: t))
        tm.needs_at_refresh.return_value = False
        balancer = LoadBalancer(tm)
        balancer._check_extension_route = AsyncMock(return_value=(True, ""))
        balancer.mark_quota_exhausted(accounts[2], "video", "PUBLIC_ERROR_INSUFFICIENT_CREDITS")
        self.assertFalse(balancer.quota_blocked(3, 10, "image"))
        balancer.mark_quota_exhausted(accounts[2], "image", "PUBLIC_ERROR_QUOTA_EXCEEDED")
        for video in (False, True):
            for _ in range(12):
                chosen = await balancer.select_token(for_image_generation=not video, for_video_generation=video)
                self.assertNotEqual(chosen.id, 3)
        accounts[2].credits = 30
        self.assertFalse(balancer.quota_blocked(3, 30, "video"))
        self.assertFalse(balancer.quota_blocked(3, 30, "image"))
        for token in accounts:
            balancer.mark_quota_exhausted(token, "video", "insufficient credits")
        self.assertIsNone(await balancer.select_token(for_video_generation=True))
        self.assertIn("semua akun", await balancer.get_unavailable_reason(for_video_generation=True))

    async def test_generic_rate_limit_does_not_mark_credit_exhaustion_or_trigger_switching(self):
        self.assertFalse(is_quota_error("429 RESOURCE_EXHAUSTED: too many requests"))
        self.assertFalse(is_quota_error("reCAPTCHA token failed"))
        self.assertTrue(is_quota_error("PUBLIC_ERROR_QUOTA_EXCEEDED"))

    async def test_last_video_credit_usage_is_read_from_persisted_logs(self):
        import json
        from src.core.models import RequestLog
        with tempfile.TemporaryDirectory() as folder:
            db = Database(str(Path(folder) / "test.db"))
            await db.init_db()
            token_id = await db.add_token(Token(st="test", email="account@example.test"))
            await db.add_request_log(RequestLog(token_id=token_id, operation="generate_video", status_code=200, duration=1.0,
                response_body=json.dumps({"credits":{"before":50,"after":32,"used":18,"model":"omni","checkedAt":1}})))
            rows = await db.get_all_tokens_with_stats()
            self.assertEqual(json.loads(rows[0]["last_video_credits"])["used"], 18)

    async def test_embedded_browser_pool_can_persist_more_than_twenty_accounts(self):
        with tempfile.TemporaryDirectory() as folder:
            db = Database(str(Path(folder) / "test.db"))
            await db.init_db()
            with patch("src.core.database.paths.is_embedded", return_value=True):
                await db.update_captcha_config(browser_count=25)
                self.assertEqual((await db.get_captcha_config()).browser_count, 25)

    async def test_scheduler_reserves_all_connected_accounts_for_images_and_videos(self):
        for video in (False, True):
            accounts = [Token(id=i, st="test", email=f"account{i}@example.test") for i in range(1, 26)]
            token_manager = Mock()
            token_manager.get_active_tokens = AsyncMock(return_value=accounts)
            token_manager.ensure_valid_token = AsyncMock(side_effect=lambda token: token)
            token_manager.needs_at_refresh.return_value = False
            manager = ConcurrencyManager()
            balancer = LoadBalancer(token_manager, manager)
            balancer._check_extension_route = AsyncMock(return_value=(True, ""))
            with patch("src.services.concurrency_manager.is_embedded", return_value=True):
                selected = await asyncio.gather(*(balancer.select_token(
                    for_image_generation=not video, for_video_generation=video, reserve=True
                ) for _ in accounts))
                self.assertEqual({token.id for token in selected}, set(range(1, 26)))
                self.assertIsNone(await balancer.select_token(for_image_generation=True, reserve=True))
                self.assertIsNone(await balancer.select_token(for_video_generation=True, reserve=True))
                release = manager.release_video if video else manager.release_image
                await release(2)
                self.assertEqual((await balancer.select_token(for_image_generation=True, reserve=True)).id, 2)

    async def test_three_accounts_overlap_but_each_account_has_one_slot(self):
        manager = ConcurrencyManager()
        with patch("src.services.concurrency_manager.is_embedded", return_value=True):
            self.assertEqual(await asyncio.gather(*(manager.acquire_image(i) for i in (1, 2, 3))), [True] * 3)
            self.assertFalse(await manager.acquire_image(1))
            self.assertFalse(await manager.can_use_image(1))
            self.assertEqual(await manager.get_image_remaining(1), 0)
            await manager.release_image(1)
            self.assertTrue(await manager.acquire_image(1))
            self.assertFalse(await manager.acquire_video(1))
            self.assertFalse(await manager.can_use_video(1))
            self.assertEqual(await manager.get_video_remaining(1), 0)
            await manager.release_image(1)
            self.assertTrue(await manager.acquire_video(1))
            self.assertFalse(await manager.acquire_video(1))
            self.assertFalse(await manager.acquire_image(1))
            await manager.release_video(1)
            self.assertTrue(await manager.acquire_image(1))

    async def test_standalone_keeps_configured_limits(self):
        manager = ConcurrencyManager()
        with patch("src.services.concurrency_manager.is_embedded", return_value=False):
            self.assertTrue(await manager.acquire_image(1))
            self.assertTrue(await manager.acquire_image(1))
            await manager.reset_token(2, image_concurrency=1)
            self.assertTrue(await manager.acquire_image(2))
            self.assertFalse(await manager.acquire_image(2))


if __name__ == "__main__":
    unittest.main()
