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


class AccountConcurrencyTest(unittest.IsolatedAsyncioTestCase):
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
