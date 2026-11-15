@echo off
setlocal
rem De Story (Tauri v2 + Rust) - klik 2x file ini untuk membuka aplikasi di Windows.
rem by KrisnaZxx
cd /d "%~dp0"
title De Story

if exist "src-tauri\target\release\de-story.exe" (
  start "" "src-tauri\target\release\de-story.exe"
  exit /b 0
)

if exist "src-tauri\target\release\bang-story.exe" (
  start "" "src-tauri\target\release\bang-story.exe"
  exit /b 0
)

call npm run tauri:dev
exit /b 0
