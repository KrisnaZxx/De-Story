import type { EditorSettings, WordTiming } from './types'

export function effectiveDurationMs(clipMs: number, audioMs?: number | null): number {
  return Math.max(500, Number.isFinite(clipMs) ? clipMs : 500, Number.isFinite(audioMs) ? audioMs ?? 0 : 0)
}

export function captionWords(editor: EditorSettings, clipId: string, audioId: string | undefined, original: WordTiming[], duration: number): WordTiming[] {
  const edit = editor.captionEdits?.[clipId]
  const words = edit && edit.audioAssetId === audioId ? edit.words : original
  const offset = Number.isFinite(editor.captionOffsetSec) ? Math.max(-30, Math.min(30, editor.captionOffsetSec ?? 0)) : 0
  return words.filter((w) => Number.isFinite(w.start) && Number.isFinite(w.end) && w.text.trim()).map((w) => ({ ...w, start: Math.max(0, Math.min(duration, w.start + offset)), end: Math.max(0, Math.min(duration, w.end + offset)) })).filter((w) => w.end > w.start).sort((a, b) => a.start - b.start)
}

export function editCaptionText(words: WordTiming[], text: string, duration: number): WordTiming[] {
  const tokens = text.trim().split(/\s+/).filter(Boolean)
  if (!tokens.length) return []
  if (tokens.length === words.length) return words.map((w, i) => ({ ...w, text: tokens[i] }))
  const start = words[0]?.start ?? 0
  const end = Math.max(start + 0.05, words.at(-1)?.end ?? duration)
  const total = tokens.reduce((n, t) => n + Math.max(1, t.length), 0)
  let at = start
  return tokens.map((text) => { const next = at + (end - start) * Math.max(1, text.length) / total; const word = { text, start: at, end: next }; at = next; return word })
}

export function shiftCaptionRange(words: WordTiming[], from: number, count: number, delta: number, duration: number): WordTiming[] {
  const selected = words.slice(from, from + count)
  if (!selected.length || !Number.isFinite(delta)) return words
  const lower = from > 0 ? words[from - 1].end : 0
  const upper = from + count < words.length ? words[from + count].start : duration
  if (upper < lower) return words
  const shift = Math.max(lower - selected[0].start, Math.min(upper - selected.at(-1)!.end, delta))
  return words.map((w, i) => i >= from && i < from + count ? { ...w, start: w.start + shift, end: w.end + shift } : w)
}
