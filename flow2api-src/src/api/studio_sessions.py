"""Authenticated session controls used by the De Story settings page."""
from typing import Literal, Optional
from fastapi import APIRouter, Depends, HTTPException
from pydantic import BaseModel, Field
from .admin import verify_admin_token
from . import admin
from ..services.managed_sessions import managed_sessions

router = APIRouter()


class SessionAction(BaseModel):
    action: Literal["add", "open", "complete", "close", "renew", "renew-all", "attach-all", "remove", "config"]
    profileId: Optional[str] = None
    tokenId: Optional[int] = Field(None, gt=0)
    autoRefresh: Optional[bool] = None
    intervalMinutes: Optional[int] = Field(None, ge=10, le=60)


@router.get("/api/studio/sessions")
async def sessions(_token=Depends(verify_admin_token)):
    return await managed_sessions.snapshot(admin.token_manager)


@router.post("/api/studio/sessions")
async def action(request: SessionAction, _token=Depends(verify_admin_token)):
    manager = admin.token_manager
    try:
        if request.action == "config":
            await manager.db.update_token_refresh_config(enabled=request.autoRefresh, refresh_interval_minutes=request.intervalMinutes)
            if request.intervalMinutes is not None:
                await manager.db.update_all_tokens_refresh_interval(request.intervalMinutes)
        elif request.action in ("renew-all", "attach-all"):
            if not managed_sessions.bulk_running:
                managed_sessions.schedule(managed_sessions.renew_all(manager, request.action == "attach-all"))
        elif request.action == "add":
            token = await manager.db.get_token(request.tokenId) if request.tokenId else None
            if request.tokenId and not token:
                raise ValueError("Akun Flow tidak ditemukan")
            managed_sessions.attach(token)
        else:
            profile = managed_sessions.profile(request.profileId)
            if request.action == "open":
                await managed_sessions.open_login(manager, profile["id"])
            elif request.action == "complete":
                await managed_sessions.complete(manager, profile["id"])
            elif request.action == "close":
                if profile["id"] in managed_sessions.busy:
                    raise ValueError("Profil sedang diproses")
                await managed_sessions.close(profile["id"])
                managed_sessions.changed(profile, "idle")
            elif request.action == "renew":
                if not profile.get("tokenId"):
                    raise ValueError("Hubungkan akun melalui Buka profil lalu Selesai login dahulu")
                if profile["state"] != "refreshing":
                    managed_sessions.changed(profile, "refreshing")
                    managed_sessions.schedule(managed_sessions.renew(manager, profile["tokenId"]))
            elif request.action == "remove":
                if profile["id"] in managed_sessions.busy:
                    raise ValueError("Profil sedang diproses")
                await managed_sessions.close(profile["id"])
                # Forget the binding, preserving Chrome's files and the existing Flow account.
                managed_sessions.records.pop(profile["id"])
                managed_sessions._save()
        return await managed_sessions.snapshot(manager)
    except (ValueError, RuntimeError) as exc:
        raise HTTPException(status_code=400, detail=str(exc)) from None
