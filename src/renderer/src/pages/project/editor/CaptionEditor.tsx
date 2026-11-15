import { useEffect, useState } from 'react'
import { captionWords, editCaptionText, effectiveDurationMs } from '@shared/editing'
import type { WordTiming } from '@shared/types'
import { useProject } from '../../../store/project'
import { Button, inputCls } from '../../../components/ui'

export function saveCaptionWords(clipId: string, audioAssetId: string, words: WordTiming[]): void {
  const s = useProject.getState()
  s.updateEditor({ captionEdits: { ...s.project!.editor.captionEdits, [clipId]: { audioAssetId, words } } })
}

export function CaptionEditor() {
  const { project, clips, assets, selectedClipId, updateEditor } = useProject()
  const [expanded, setExpanded] = useState(false)
  const [draft, setDraft] = useState('')
  const [focused, setFocused] = useState(false)
  const e = project?.editor ?? {} as import('@shared/types').EditorSettings
  const clip = clips.find(c => c.id === selectedClipId)
  const audio = clip?.audioAssetId ? assets[clip.audioAssetId] : null
  const duration = clip ? effectiveDurationMs(clip.durationMs, audio?.durationMs) / 1000 : 0
  const words = clip && audio ? captionWords({ ...e, captionOffsetSec: 0 }, clip.id, audio.id, audio.meta.words ?? [], duration) : []
  const text = words.map(w => w.text).join(' ')
  useEffect(() => { setDraft(text); setFocused(false) }, [clip?.id, audio?.id])
  useEffect(() => { if (!focused) setDraft(text) }, [text, focused])
  if (!project) return null
  const save = (next: WordTiming[]): void => { if (clip && audio) saveCaptionWords(clip.id, audio.id, next) }
  return <div className="flex flex-col gap-3 border-b border-line p-4 text-sm">
    <strong>Edit caption · {clip ? String(clip.sort + 1).padStart(2, '0') : 'Pilih klip'}</strong>
    <label className="flex flex-col gap-1">Geser seluruh caption (detik)
      <input className={inputCls} aria-label="Offset caption" type="number" min={-30} max={30} step={0.1} value={e.captionOffsetSec ?? 0} onChange={ev => updateEditor({ captionOffsetSec: Math.max(-30, Math.min(30, Number(ev.target.value))) })} />
      <span className="text-xs text-muted">Nilai negatif memajukan caption. Posisi teks bisa ditarik langsung pada preview.</span>
    </label>
    {audio && clip && <>
      <textarea aria-label="Teks caption klip" className={inputCls + ' min-h-24 py-2'} value={draft} onFocus={() => setFocused(true)} onChange={ev => setDraft(ev.target.value)} onBlur={() => { save(editCaptionText(words, draft, duration)); setFocused(false) }} />
      <div className="flex flex-wrap gap-2">
        <Button size="sm" onClick={() => setExpanded(!expanded)}>{expanded ? 'Tutup waktu kata' : 'Edit waktu per kata'}</Button>
        <Button size="sm" onClick={() => { const next = { ...e.captionEdits }; delete next[clip.id]; updateEditor({ captionEdits: next }) }}>Pulihkan caption klip</Button>
      </div>
      {expanded && <div className="flex max-h-64 flex-col gap-1 overflow-y-auto">
        <div className="grid grid-cols-[minmax(0,1fr)_72px_72px] gap-1 text-xs text-muted"><span>Kata</span><span>Mulai (s)</span><span>Akhir (s)</span></div>
        {words.map((w, i) => <div key={i} className="grid grid-cols-[minmax(0,1fr)_72px_72px] gap-1">
          <input aria-label={`Kata ${i + 1}`} className="min-w-0 rounded border border-line p-1" value={w.text} onChange={ev => save(words.map((x, n) => n === i ? { ...x, text: ev.target.value } : x))} />
          {(['start', 'end'] as const).map(field => <input key={field} aria-label={`${field} kata ${i + 1}`} className="min-w-0 rounded border border-line p-1" type="number" min={0} max={duration} step={0.05} value={Number(w[field].toFixed(3))} onChange={ev => { const v = Math.max(0, Math.min(duration, Number(ev.target.value))); save(words.map((x, n) => n === i ? { ...x, [field]: field === 'start' ? Math.max(0, Math.min(v, x.end - 0.01)) : Math.min(duration, Math.max(v, x.start + 0.01)) } : x)) }} />)}
        </div>)}
      </div>}
    </>}
  </div>
}
