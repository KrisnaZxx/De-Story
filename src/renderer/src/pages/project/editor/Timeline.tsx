import { memo, useEffect, useLayoutEffect, useRef, useState, type PointerEvent } from 'react'
import { ChevronLeft, ChevronRight, Film, Image as ImageIcon, Layers, Mic, Music, Type, ZoomIn } from 'lucide-react'
import { captionWords, shiftCaptionRange } from '@shared/editing'
import { useProject } from '../../../store/project'
import { saveCaptionWords } from './CaptionEditor'
import type { CaptionChunk } from '@shared/captions'
import { overlayEnd } from '@shared/overlays'
import type { Asset, Overlay } from '@shared/types'
import { cx } from '../../../components/ui'
import { usePlayback, type TimelineItem } from './engine'
import { TransitionButton } from './TransitionButton'
import { Waveform } from './Waveform'

const LABEL_W = 140
const TICK_STEPS = [1, 2, 5, 10, 15, 30, 60, 120]

function Playhead({ pps }: { pps: number }) {
  const t = usePlayback((s) => s.t)
  return (
    <div className="pointer-events-none absolute bottom-0 top-0 z-10 w-0.5 bg-accent" style={{ left: t * pps }}>
      <span className="absolute -left-[6px] -top-0.5 size-3.5 rounded-[3px] border-[1.5px] border-ink bg-accent" />
    </div>
  )
}

const Tracks = memo(function Tracks({
  items,
  chunks,
  music,
  total,
  pps,
  selectedId,
  onSelect,
  overlays,
  assets,
  selectedOverlayId,
  onSelectOverlay, onCaption
}: {
  items: TimelineItem[]
  chunks: CaptionChunk[]
  music: Asset | null
  total: number
  pps: number
  selectedId: string | null
  onSelect: (clipId: string, start: number) => void
  overlays: Overlay[]
  assets: Record<string, Asset>
  selectedOverlayId: string | null
  onCaption: (id: string) => void
  onSelectOverlay: (id: string, start: number) => void
}) {
  const captionDrag = useRef<{ x: number; chunk: CaptionChunk; from: number; words: import('@shared/types').WordTiming[]; audioId: string; duration: number } | null>(null)
  const step = TICK_STEPS.find((s) => s * pps >= 70) ?? 120
  const ticks: number[] = []
  for (let s = 0; s <= total + 0.001; s += step) ticks.push(s)
  return (
    <>
      <div className="relative h-[22px] border-b border-line">
        {ticks.map((s) => (
          <span
            key={s}
            className="absolute top-0 h-[21px] border-l border-line-2 pl-1 font-mono text-[11px] text-muted"
            style={{ left: s * pps }}
          >
            {Math.floor(s / 60)}:{String(Math.round(s % 60)).padStart(2, '0')}
          </span>
        ))}
      </div>
      <div className="relative h-16">
        {items.map((it) => (
          <button
            key={it.clip.id}
            type="button"
            onPointerDown={(e) => e.stopPropagation()}
            onClick={() => onSelect(it.clip.id, it.start)}
            aria-label={`Klip ${it.index + 1} ${it.clip.title}`}
            aria-pressed={it.clip.id === selectedId}
            className={cx(
              'absolute top-0 flex h-16 flex-col justify-between overflow-hidden rounded-lg bg-sand bg-cover bg-center p-1.5 text-left',
              it.clip.id === selectedId ? 'border-2 border-accent' : 'border border-ink'
            )}
            style={{
              left: it.start * pps + 1,
              width: Math.max(8, (it.end - it.start) * pps - 3),
              backgroundImage: it.image ? `url("${it.image.url}")` : undefined
            }}
          >
            <span className="flex items-center gap-1 self-start">
              <span className="rounded-[5px] border border-ink bg-surface px-1 font-mono text-[11px]">{String(it.index + 1).padStart(2, '0')}</span>
              {it.clip.motionType === 'video' && it.video && (
                <span className="flex size-[18px] items-center justify-center rounded-[5px] bg-ink text-paper">
                  <Film className="size-3" />
                </span>
              )}
            </span>
            <span className="max-w-full truncate rounded bg-surface/85 px-1 text-xs font-semibold">{it.clip.title}</span>
          </button>
        ))}
        {/* One button on each joint: the transition belongs to the clip before it. */}
        {items.slice(1).map((it, i) => (
          (it.start - items[i].start) * pps >= 65 || items[i].clip.id === selectedId ?
          <TransitionButton key={`tr-${items[i].clip.id}`} clip={items[i].clip} next={it.clip} x={it.start * pps} /> : null
        ))}
      </div>
      <div className="relative h-[30px]">
        {chunks.map((c, i) => (
          <span
            key={i}
            role="button" tabIndex={0}
            onKeyDown={e => { if ((e.key === 'Enter' || e.key === ' ') && c.clipId) { e.preventDefault(); usePlayback.getState().seek(c.start + .01); onCaption(c.clipId) } }}
            onPointerDown={e => {
              e.stopPropagation(); if (!c.clipId) return
              usePlayback.getState().pause(); usePlayback.getState().seek(c.start + .01)
              onCaption(c.clipId)
              const it = items.find(it => it.clip.id === c.clipId), state = useProject.getState()
              if (!it?.audio || !state.project) return
              const words = captionWords({ ...state.project.editor, captionOffsetSec: 0 }, c.clipId, it.audio.id, it.audio.meta.words ?? [], it.end - it.start)
              const offset = state.project.editor.captionOffsetSec ?? 0
              const from = words.findIndex(w => w.text === c.words[0]?.text && Math.abs(w.start + offset + it.start - c.words[0].start) < .002)
              if (from < 0) return
              captionDrag.current = { x: e.clientX, chunk: c, from, words, audioId: it.audio.id, duration: it.end - it.start }
              state.beginEdit(); e.currentTarget.setPointerCapture(e.pointerId)
            }}
            onPointerMove={e => { e.stopPropagation(); const d = captionDrag.current; if (d?.chunk.clipId) saveCaptionWords(d.chunk.clipId, d.audioId, shiftCaptionRange(d.words, d.from, d.chunk.words.length, (e.clientX - d.x) / pps, d.duration)) }}
            onPointerUp={e => { e.stopPropagation(); captionDrag.current = null; useProject.getState().endEdit() }}
            onLostPointerCapture={() => { captionDrag.current = null; useProject.getState().endEdit() }}
            className="absolute top-0 flex h-[30px] cursor-ew-resize touch-none items-center overflow-hidden whitespace-nowrap rounded-md border border-[#E3C46A] bg-sun-soft px-1.5 text-xs text-sun-ink"
            style={{ left: c.start * pps + 1, width: Math.max(4, (c.end - c.start) * pps - 2) }}
            title={c.words.map((w) => w.text).join(' ')}
          >
            {c.words.map((w) => w.text).join(' ')}
          </span>
        ))}
      </div>
      <div className="relative h-10">
        {items.map((it) =>
          it.audio ? (
            <span
              key={it.audio.id}
              className="absolute top-0 flex h-10 items-center overflow-hidden rounded-md border border-line-2 bg-[#EFEAE2] px-1"
              style={{ left: it.start * pps + 1, width: Math.max(4, ((it.audio.durationMs ?? 0) / 1000) * pps - 2) }}
            >
              <Waveform url={it.audio.url} className="h-8 w-full" />
            </span>
          ) : null
        )}
      </div>
      <div className="relative h-[30px]">
        {music && (
          <span
            className="absolute left-px top-0 flex h-[30px] items-center gap-2 overflow-hidden whitespace-nowrap rounded-md border border-[#9CC596] bg-ok-soft px-2.5 text-xs text-ok-ink"
            style={{ width: Math.max(10, total * pps - 2) }}
          >
            <Music className="size-3.5" />
            {music.prompt ?? 'Musik latar'}
          </span>
        )}
      </div>
      <div className="relative h-[30px]">
        {overlays.map((o) => {
          const start = Math.min(o.start, total)
          const end = overlayEnd(o, total)
          if (end <= start) return null
          const on = o.id === selectedOverlayId
          const label = o.kind === 'text' ? o.text.split('\n')[0] || 'Teks' : (assets[o.assetId]?.prompt ?? 'Gambar')
          return (
            <button
              key={o.id}
              type="button"
              onPointerDown={(e) => e.stopPropagation()}
              onClick={() => onSelectOverlay(o.id, start)}
              aria-pressed={on}
              title={label}
              className={cx(
                'absolute top-0 flex h-[30px] items-center gap-1.5 overflow-hidden whitespace-nowrap rounded-md px-2 text-xs font-medium',
                o.kind === 'image' ? 'bg-info-soft text-info-ink' : 'bg-accent-soft text-accent-dark',
                on ? 'z-[1] border-2 border-accent' : 'border border-line-3'
              )}
              style={{ left: start * pps + 1, width: Math.max(10, (end - start) * pps - 2) }}
            >
              {o.kind === 'image' ? <ImageIcon className="size-3.5 shrink-0" /> : <Type className="size-3.5 shrink-0" />}
              <span className="truncate">{label}</span>
            </button>
          )
        })}
      </div>
    </>
  )
})

export function Timeline({
  items,
  chunks,
  music,
  total,
  selectedId,
  onSelect,
  overlays,
  assets,
  selectedOverlayId,
  onSelectOverlay, onCaption
}: {
  items: TimelineItem[]
  chunks: CaptionChunk[]
  music: Asset | null
  total: number
  selectedId: string | null
  onSelect: (clipId: string) => void
  overlays: Overlay[]
  assets: Record<string, Asset>
  selectedOverlayId: string | null
  onCaption: (id: string) => void
  onSelectOverlay: (id: string) => void
}) {
  const scroller = useRef<HTMLDivElement>(null)
  const [width, setWidth] = useState(1000)
  const [zoom, setZoom] = useState(1)
  const [left, setLeft] = useState(0)
  const t = usePlayback(s => s.t)
  const playing = usePlayback(s => s.playing)
  const seek = usePlayback((s) => s.seek)

  useLayoutEffect(() => {
    const el = scroller.current
    if (!el) return
    const ro = new ResizeObserver(() => setWidth(el.clientWidth))
    ro.observe(el)
    return () => ro.disconnect()
  }, [])

  const pps = Math.max(2, ((width - 16) / Math.max(total, 1)) * zoom)

  const maxScroll = Math.max(0, total * pps + 16 - width)
  const pan = (x: number): void => { if (scroller.current) scroller.current.scrollLeft = Math.max(0, Math.min(maxScroll, x)) }
  useEffect(() => {
    const el = scroller.current
    if (el && playing && (t * pps < el.scrollLeft || t * pps > el.scrollLeft + width - 24)) pan(t * pps - width * .2)
  }, [t, playing, pps, width])
  const seekFrom = (e: PointerEvent<HTMLDivElement>): void => {
    const el = scroller.current!
    const x = e.clientX - el.getBoundingClientRect().left + el.scrollLeft - 8
    seek(Math.min(total, Math.max(0, x / pps)))
  }

  return (
    <section className="flex min-h-0 min-w-0 flex-col border-t border-line-2 bg-surface">
      <div className="flex h-8 shrink-0 items-center gap-3 border-b border-line px-4 text-[13px] text-ink-2">
        <span className="truncate text-xs">Pilih klip / caption untuk mengedit. Tarik caption untuk menggeser waktunya.</span>
        <span className="flex-1" />
        <label className="flex items-center gap-2">
          <ZoomIn className="size-4" />
          Zoom
          <input type="range" min={1} max={24} step={0.25} value={zoom} onChange={(e) => { const el = scroller.current; const center = ((el?.scrollLeft ?? 0) + width / 2) / pps; const z = Number(e.target.value); setZoom(z); requestAnimationFrame(() => { if (el) el.scrollLeft = Math.max(0, center * Math.max(2, (width - 16) / Math.max(total, 1) * z) - width / 2) }) }} aria-label="Zoom timeline" className="w-32" />
        </label>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto"><div className="grid min-w-0 grid-cols-[140px_minmax(0,1fr)] py-1.5 pr-4">
        <div className="flex flex-col gap-2 pt-[30px] text-[13px] font-semibold" style={{ width: LABEL_W }}>
          <span className="flex h-16 items-center gap-2 pl-4">
            <ImageIcon className="size-4" />
            Klip
          </span>
          <span className="flex h-[30px] items-center gap-2 pl-4">
            <Type className="size-4" />
            Caption
          </span>
          <span className="flex h-10 items-center gap-2 pl-4">
            <Mic className="size-4" />
            Suara TTS
          </span>
          <span className="flex h-[30px] items-center gap-2 pl-4">
            <Music className="size-4" />
            Musik
          </span>
          <span className="flex h-[30px] items-center gap-2 pl-4">
            <Layers className="size-4" />
            Overlay
          </span>
        </div>
        <div
          ref={scroller}
          className="scroll-thin relative min-w-0 overflow-x-auto overflow-y-hidden"
          onScroll={e => setLeft(e.currentTarget.scrollLeft)}
          onWheel={e => { if (e.shiftKey || Math.abs(e.deltaX) > Math.abs(e.deltaY)) { e.preventDefault(); pan(left + (e.deltaX || e.deltaY)) } }}
          onPointerDown={(e) => {
            e.currentTarget.setPointerCapture(e.pointerId)
            seekFrom(e)
          }}
          onPointerMove={(e) => e.buttons === 1 && seekFrom(e)}
        >
          <div className="relative flex flex-col gap-2 pl-2" style={{ width: total * pps + 16 }}>
            <Tracks
              onCaption={onCaption}
              items={items}
              chunks={chunks}
              music={music}
              total={total}
              pps={pps}
              selectedId={selectedId}
              onSelect={(id, start) => {
                onSelect(id)
                seek(start)
              }}
              overlays={overlays}
              assets={assets}
              selectedOverlayId={selectedOverlayId}
              onSelectOverlay={(id, start) => {
                onSelectOverlay(id)
                seek(start + 0.01)
              }}
            />
            <div className="pointer-events-none absolute inset-y-0 left-2 right-0">
              <Playhead pps={pps} />
            </div>
          </div>
        </div>
      </div></div>
      <div className="flex h-7 shrink-0 items-center gap-2 border-t border-line px-3">
        <span className="text-xs text-muted">Geser timeline</span>
        <button type="button" aria-label="Timeline ke kiri" disabled={left <= 0} onClick={() => pan(left - width * .7)}><ChevronLeft className="size-4" /></button>
        <input className="min-w-0 flex-1" aria-label="Posisi horizontal semua track" type="range" min={0} max={Math.max(1, maxScroll)} value={Math.min(left, maxScroll)} step={1} disabled={maxScroll <= 0} onChange={e => pan(Number(e.target.value))} />
        <button type="button" aria-label="Timeline ke kanan" disabled={left >= maxScroll} onClick={() => pan(left + width * .7)}><ChevronRight className="size-4" /></button>
      </div>
    </section>
  )
}
