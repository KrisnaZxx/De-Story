import { useEffect, useState } from 'react'
import { cx } from '../../components/ui'
import { useProject } from '../../store/project'
import { useOverlayUi, usePlayback, usePlaybackEngine, useTimeline } from './editor/engine'
import { INSPECTOR_TABS, Inspector, type InspectorTab } from './editor/Inspector'
import { Player } from './editor/Player'
import { Timeline } from './editor/Timeline'

export function EditorStep() {
  const { project, assets, selectedClipId, select, clips } = useProject()
  const { items, total, chunks } = useTimeline()
  const [theater, setTheater] = useState(false)
  const [tab, setTab] = useState<InspectorTab>('klip')
  const selectedOverlayId = useOverlayUi((s) => s.selectedId)
  const selectOverlay = useOverlayUi((s) => s.select)
  const music = project?.editor.musicAssetId ? (assets[project.editor.musicAssetId] ?? null) : null
  usePlaybackEngine(items, total, music, project?.editor.musicVolumeDb ?? -18, project?.editor.duckMusic ?? true)

  useEffect(() => {
    const onKey = (e: KeyboardEvent): void => {
      const tag = (e.target as HTMLElement)?.tagName
      if (e.code === 'Space' && tag !== 'INPUT' && tag !== 'TEXTAREA' && tag !== 'SELECT') {
        e.preventDefault()
        const s = usePlayback.getState()
        if (!s.playing && s.t >= total - 0.05) s.seek(0)
        s.toggle()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [total])

  useEffect(() => {
    const s = usePlayback.getState(); s.pause()
    const state = useProject.getState()
    let start = 0
    for (const c of state.clips) { if (c.id === state.selectedClipId) break; start += Math.max(c.durationMs, state.assets[c.audioAssetId ?? '']?.durationMs ?? 0) / 1000 }
    s.seek(start)
    return () => usePlayback.getState().pause()
  }, [project?.id])
  useEffect(() => { const s = usePlayback.getState(); if (s.t > total) s.seek(total) }, [total])
  if (!project) return null
  const selIndex = clips.findIndex((c) => c.id === selectedClipId)
  const sel = selIndex >= 0 ? clips[selIndex] : null

  return (
    <div className="grid h-full min-h-0 min-w-0" style={{ gridTemplateRows: theater ? 'minmax(0,1fr)' : 'minmax(180px,1fr) minmax(200px,30vh)' }}>
      <div className="grid min-h-0 min-w-0" style={{ gridTemplateColumns: theater ? 'minmax(0,1fr)' : '64px minmax(0,1fr) minmax(280px,25vw)' }}>
        {!theater && <nav aria-label="Panel editor" className="flex flex-col items-center gap-1 border-r border-line bg-surface py-3">
          {INSPECTOR_TABS.map((t) => {
            const Icon = t.icon
            const on = tab === t.id
            return (
              <button
                key={t.id}
                type="button"
                aria-pressed={on}
                onClick={() => setTab(t.id)}
                className={cx(
                  'flex h-[62px] w-[56px] flex-col items-center justify-center gap-1 rounded-xl text-xs',
                  on ? 'border-[1.5px] border-line-2 bg-sun-soft font-semibold text-ink' : 'font-medium text-ink-2 hover:bg-sand'
                )}
              >
                <Icon className="size-5" />
                {t.label}
              </button>
            )
          })}
        </nav>}
        <Player
          items={items}
          total={total}
          chunks={chunks}
          aspect={project.aspectRatio}
          editor={project.editor}
          assets={assets}
          editOverlays={tab === 'overlay'}
          editCaptions={tab === 'caption'}
          theater={theater}
          onTheater={() => setTheater(!theater)}
        />
        {!theater && <Inspector tab={tab} clip={sel} index={selIndex} total={total} />}
      </div>
      {!theater && <Timeline
        onCaption={(id) => { select(id); setTab('caption') }}
        items={items}
        chunks={project.editor.captionStyle === 'none' ? [] : chunks}
        music={music}
        total={total}
        selectedId={selectedClipId}
        onSelect={(id) => {
          select(id)
          setTab('klip')
        }}
        overlays={project.editor.overlays ?? []}
        assets={assets}
        selectedOverlayId={selectedOverlayId}
        onSelectOverlay={(id) => {
          selectOverlay(id)
          setTab('overlay')
        }}
      />}
    </div>
  )
}
