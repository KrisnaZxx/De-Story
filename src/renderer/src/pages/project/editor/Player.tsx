import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type PointerEvent } from 'react'
import { Maximize, Minimize, PanelRightClose, Pause, Play, SkipBack, SkipForward } from 'lucide-react'
import {
  CAPTION_BOX_PAD,
  CAPTION_BOX_RADIUS,
  CAPTION_MAX_WIDTH,
  POPPINS_CELL,
  captionScaleOf,
  captionAt,
  displayWord,
  layoutCaptionLines,
  resolveCaption,
  type CaptionChunk
} from '@shared/captions'
import { TRANSITION_SEC, cameraAt, cameraCss } from '@shared/motion'
import { fitPreviewBox } from '@shared/preview'
import { anchorOffset, contrastColor, overlayVisible, textEffects } from '@shared/overlays'
import type { AspectRatio, Asset, EditorSettings, Overlay } from '@shared/types'
import { IconButton, cx } from '../../../components/ui'
import { timecode } from '../../../lib/format'
import { useProject } from '../../../store/project'
import { itemAt, updateOverlay, useOverlayUi, usePlayback, type TimelineItem } from './engine'

function hexA(hex: string, a: number): string {
  const h = hex.replace('#', '')
  return `rgba(${parseInt(h.slice(0, 2), 16)}, ${parseInt(h.slice(2, 4), 16)}, ${parseInt(h.slice(4, 6), 16)}, ${a})`
}

function Frame({ item, local, playing, style }: { item: TimelineItem; local: number; playing: boolean; style?: CSSProperties }) {
  const video = useRef<HTMLVideoElement>(null)
  const useVideo = item.clip.motionType === 'video' && !!item.video

  useEffect(() => {
    const v = video.current
    if (!v) return
    if (Number.isFinite(v.duration) && local >= v.duration) { if (!v.paused) v.pause(); if (Math.abs(v.currentTime - Math.max(0, v.duration - .04)) > .05) v.currentTime = Math.max(0, v.duration - .04); return }
    if (playing) {
      if (v.paused) {
        v.currentTime = Math.min(local, (v.duration || Infinity) - 0.05)
        void v.play().catch(() => {})
      } else if (Math.abs(v.currentTime - local) > 0.35 && local < (v.duration || Infinity)) v.currentTime = local
    } else {
      if (!v.paused) v.pause()
      if (Math.abs(v.currentTime - local) > 0.04) v.currentTime = Math.min(local, Math.max(0, (v.duration || Infinity) - 0.05))
    }
  })

  const own = item.end - item.start
  const cam = cameraAt(item.clip.cameraPreset, item.clip.motionStrength, Math.min(1, local / own), local)
  return (
    <div className="absolute inset-0 overflow-hidden bg-sand" style={style}>
      {useVideo ? (
        <video ref={video} src={item.video!.url} poster={item.image?.url} muted playsInline className="size-full object-cover" />
      ) : item.image ? (
        <img
          src={item.image.url}
          alt=""
          draggable={false}
          className="absolute inset-0 size-full object-cover will-change-transform"
          style={{ transform: cameraCss(cam) }}
        />
      ) : (
        <div className="flex size-full items-center justify-center text-sm text-muted">Klip {item.index + 1} belum punya gambar</div>
      )}
    </div>
  )
}

let measureCtx: CanvasRenderingContext2D | null = null

/** Text width in pixels with the caption font, for the same line breaks the export uses. */
function textWidth(font: string, text: string): number {
  measureCtx ??= document.createElement('canvas').getContext('2d')
  if (!measureCtx) return text.length * 10
  measureCtx.font = font
  return measureCtx.measureText(text).width
}

function Caption({ chunks, t, editor, width, height, editable }: { chunks: CaptionChunk[]; t: number; editor: EditorSettings; width: number; height: number; editable: boolean }) {
  const drag = useRef<{ px: number; py: number; x: number; y: number } | null>(null)
  const style = resolveCaption(editor)
  if (!style) return null
  const hit = captionAt(chunks, t)
  if (!hit) return null
  const size = style.size * height * captionScaleOf(editor)
  // As in libass, the margin places the text line and the box grows outward by its padding.
  const pad = style.box ? size * CAPTION_BOX_PAD : 0
  const pos: CSSProperties =
    editor.captionAnchor ? { left: `${editor.captionAnchor.x * 100}%`, top: `${editor.captionAnchor.y * 100}%`, transform: 'translate(-50%, -50%)' } : editor.captionPosition === 'atas'
      ? { top: height * 0.07 - pad, left: '50%', transform: 'translateX(-50%)' }
      : editor.captionPosition === 'tengah'
        ? { top: '50%', left: '50%', transform: 'translate(-50%, -50%)' }
        : { bottom: height * 0.07 - pad, left: '50%', transform: 'translateX(-50%)' }
  const highlight = style.highlight.toLowerCase() !== style.color.toLowerCase()
  const family = `'${style.fontInfo.family}', sans-serif`
  const words = hit.chunk.words.map((w) => displayWord(w.text, style))
  const font = `${style.weight} ${size}px ${family}`
  const lines = layoutCaptionLines(words, (s) => textWidth(font, s), width * CAPTION_MAX_WIDTH)
  let index = 0
  return (
    <div
      className={cx("absolute select-none whitespace-nowrap text-center", editable && 'cursor-move outline outline-1 outline-accent')}
      onPointerDown={e => { if (!editable) return; e.stopPropagation(); e.preventDefault(); const r = e.currentTarget.getBoundingClientRect(); const frame = e.currentTarget.parentElement!.getBoundingClientRect(); drag.current = { px: e.clientX, py: e.clientY, x: (r.left + r.width / 2 - frame.left) / width, y: (r.top + r.height / 2 - frame.top) / height }; useProject.getState().beginEdit(); e.currentTarget.setPointerCapture(e.pointerId) }}
      onPointerMove={e => { const d = drag.current; if (d) useProject.getState().updateEditor({ captionAnchor: { x: clamp01(d.x + (e.clientX - d.px) / width), y: clamp01(d.y + (e.clientY - d.py) / height) } }) }}
      onPointerUp={() => { drag.current = null; useProject.getState().endEdit() }}
      onLostPointerCapture={() => { drag.current = null; useProject.getState().endEdit() }}
      onClick={e => { if (editable) e.stopPropagation() }}
      style={{
        ...pos,
        fontFamily: family,
        fontWeight: style.weight,
        fontSize: size,
        lineHeight: style.fontInfo.cell,
        color: style.color,
        background: style.box ? hexA(style.box, style.boxAlpha) : 'transparent',
        padding: pad,
        borderRadius: style.box ? size * CAPTION_BOX_RADIUS : 0,
        // A CSS stroke is centred on the glyph edge; doubling it leaves the same width outside as libass.
        WebkitTextStroke: style.box ? undefined : `${Math.max(1, style.outline * height) * 2}px ${style.outlineColor}`,
        paintOrder: 'stroke fill'
      }}
    >
      {lines.map((line, l) => (
        <div key={l}>
          {line.map((word, k) => {
            const i = index++
            return (
              <span key={k} style={highlight && i === hit.word ? { color: style.highlight } : undefined}>
                {word}
                {k < line.length - 1 ? ' ' : ''}
              </span>
            )
          })}
        </div>
      ))}
    </div>
  )
}

const clamp01 = (v: number): number => Math.min(1, Math.max(0, v))

/**
 * One logo or text drawn over the preview at the same spot the export uses. While the Overlay tab is open
 * it can be picked and dragged; otherwise it ignores the mouse so clicks still play and pause.
 */
function OverlayItem({ o, asset, box, editable }: { o: Overlay; asset: Asset | null; box: { w: number; h: number }; editable: boolean }) {
  const selected = useOverlayUi((s) => s.selectedId === o.id)
  const select = useOverlayUi((s) => s.select)
  const drag = useRef<{ px: number; py: number; x: number; y: number } | null>(null)
  const { ax, ay } = anchorOffset(o.anchor)

  const handlers = editable
    ? {
        onPointerDown: (e: PointerEvent<HTMLDivElement>) => {
          e.stopPropagation()
          e.preventDefault()
          select(o.id)
          useProject.getState().beginEdit()
          drag.current = { px: e.clientX, py: e.clientY, x: o.x, y: o.y }
          e.currentTarget.setPointerCapture(e.pointerId)
        },
        onPointerMove: (e: PointerEvent<HTMLDivElement>) => {
          const d = drag.current
          if (!d) return
          updateOverlay(o.id, { x: clamp01(d.x + (e.clientX - d.px) / box.w), y: clamp01(d.y + (e.clientY - d.py) / box.h) })
        },
        onPointerUp: () => {
          drag.current = null
          useProject.getState().endEdit()
        },
        onLostPointerCapture: () => { drag.current = null; useProject.getState().endEdit() },
        onClick: (e: React.MouseEvent) => e.stopPropagation()
      }
    : {}

  // libass anchors the text itself and grows the box around it, so the padding shifts the element outward.
  const pad = o.kind === 'text' ? textEffects(o.style, o.size * box.h).pad : 0
  const place: CSSProperties = {
    position: 'absolute',
    left: `${o.x * 100}%`,
    top: `${o.y * 100}%`,
    transform: `translate(calc(${-ax * 100}% + ${pad * (2 * ax - 1)}px), calc(${-ay * 100}% + ${pad * (2 * ay - 1)}px))`
  }
  const ring = editable ? (selected ? 'cursor-move outline outline-2 outline-offset-2 outline-accent' : 'cursor-move hover:outline hover:outline-1 hover:outline-offset-2 hover:outline-paper') : 'pointer-events-none'

  if (o.kind === 'image') {
    if (!asset) return null
    return (
      <div {...handlers} className={cx('select-none', ring)} style={{ ...place, width: o.width * box.w, opacity: o.opacity }}>
        <img src={asset.url} alt="" draggable={false} className="block w-full" />
      </div>
    )
  }

  const fs = o.size * box.h
  const fx = textEffects(o.style, fs)
  const contrast = contrastColor(o.color)
  return (
    <div
      {...handlers}
      className={cx('select-none', ring)}
      style={{
        ...place,
        fontFamily: 'Poppins, sans-serif',
        fontWeight: o.bold ? 700 : 500,
        fontSize: fs,
        lineHeight: POPPINS_CELL,
        color: o.color,
        whiteSpace: 'pre',
        textAlign: ax === 0 ? 'left' : ax === 1 ? 'right' : 'center',
        ...(o.style === 'box' ? { background: hexA(contrast, 0.85), padding: fx.pad } : {}),
        ...(o.style === 'outline' ? { WebkitTextStroke: `${fx.outline * 2}px ${contrast}`, paintOrder: 'stroke fill' } : {}),
        ...(o.style === 'plain' ? { textShadow: `${fx.shadow}px ${fx.shadow}px 0 rgba(0, 0, 0, 0.56)` } : {})
      }}
    >
      {o.text}
    </div>
  )
}

export function Player({
  items,
  total,
  chunks,
  aspect,
  editor,
  assets,
  editOverlays, editCaptions, theater, onTheater
}: {
  items: TimelineItem[]
  total: number
  chunks: CaptionChunk[]
  aspect: AspectRatio
  editor: EditorSettings
  assets: Record<string, Asset>
  /** The Overlay tab is open: overlays can be selected and dragged. */
  editCaptions: boolean
  theater: boolean
  onTheater(): void
  editOverlays: boolean
}) {
  const { t, playing, toggle, seek } = usePlayback()
  const [fullscreen, setFullscreen] = useState(false)
  useEffect(() => { const change = (): void => setFullscreen(!!document.fullscreenElement); document.addEventListener('fullscreenchange', change); return () => document.removeEventListener('fullscreenchange', change) }, [])
  const stage = useRef<HTMLDivElement>(null)
  const [box, setBox] = useState({ w: 800, h: 450 })
  const ratio = aspect === '9:16' ? 9 / 16 : 16 / 9

  useLayoutEffect(() => {
    const el = stage.current
    if (!el) return
    const ro = new ResizeObserver(() => {
      setBox(fitPreviewBox(el.clientWidth, el.clientHeight, ratio))
    })
    ro.observe(el)
    return () => ro.disconnect()
  }, [ratio])

  const idx = itemAt(items, t)
  const cur = items[idx]
  const prev = idx > 0 ? items[idx - 1] : null
  const local = cur ? t - cur.start : 0
  const inTransition = prev && prev.clip.transition !== 'cut' && local < TRANSITION_SEC
  const k = inTransition ? local / TRANSITION_SEC : 1
  let prevStyle: CSSProperties | undefined
  let curStyle: CSSProperties | undefined
  if (inTransition) {
    if (prev!.clip.transition === 'slideleft') {
      prevStyle = { transform: `translateX(${-k * 100}%)` }
      curStyle = { transform: `translateX(${(1 - k) * 100}%)` }
    } else if (prev!.clip.transition === 'zoomin') {
      prevStyle = { transform: `scale(${1 + k * 0.6})`, opacity: 1 - k }
    } else {
      prevStyle = { opacity: 1 - k }
    }
  }

  const shown = (editor.overlays ?? []).filter((o) => overlayVisible(o, t, total))

  const jump = (dir: -1 | 1): void => {
    const target = items[Math.min(items.length - 1, Math.max(0, idx + dir))]
    if (!target) return
    seek(dir === -1 && local > 1 ? cur.start : target.start)
  }

  return (
    <section ref={stage} className="flex min-h-0 min-w-0 flex-col items-center justify-center gap-2 overflow-hidden bg-sand p-2">
      <div
        className="relative shrink-0 overflow-hidden rounded-xl border-[1.5px] border-line-2 bg-[#0d0e11]"
        style={{ width: box.w, height: box.h }}
        onClick={toggle}
      >
        {cur && <Frame key={cur.clip.id} item={cur} local={local} playing={playing} style={curStyle} />}
        {inTransition && prev && <Frame key={`prev-${prev.clip.id}`} item={prev} local={prev.end - prev.start} playing={false} style={prevStyle} />}
        {/* Logos sit under the captions and overlay text sits above them, as in the export. */}
        {shown
          .filter((o) => o.kind === 'image')
          .map((o) => (
            <OverlayItem key={o.id} o={o} asset={o.kind === 'image' ? (assets[o.assetId] ?? null) : null} box={box} editable={editOverlays} />
          ))}
        <Caption chunks={chunks} t={t} editor={editor} width={box.w} height={box.h} editable={editCaptions} />
        {shown
          .filter((o) => o.kind === 'text')
          .map((o) => (
            <OverlayItem key={o.id} o={o} asset={null} box={box} editable={editOverlays} />
          ))}
        {!items.length && <div className="flex size-full items-center justify-center text-white">Belum ada klip</div>}
      </div>
      <div className="flex h-10 w-full max-w-[1000px] shrink-0 items-center justify-center gap-2">
        <IconButton label="Klip sebelumnya" className="border-transparent bg-transparent" onClick={() => jump(-1)}>
          <SkipBack className="size-5" />
        </IconButton>
        <button
          type="button"
          aria-label={playing ? 'Jeda' : 'Putar'}
          onClick={() => {
            if (!playing && t >= total - 0.05) seek(0)
            toggle()
          }}
          className="flex size-9 items-center justify-center rounded-full border-[1.5px] border-line-2 bg-accent-soft text-accent-ink hover:bg-accent/20"
        >
          {playing ? <Pause className="size-5" fill="currentColor" /> : <Play className="ml-0.5 size-5" fill="currentColor" />}
        </button>
        <IconButton label="Klip berikutnya" className="border-transparent bg-transparent" onClick={() => jump(1)}>
          <SkipForward className="size-5" />
        </IconButton>
        <span className="whitespace-nowrap font-mono text-sm">
          {timecode(t)} <span className="text-muted">/ {timecode(total)}</span>
        </span>
        <span className="flex-1" />
        <IconButton label={theater ? 'Tampilkan panel editor' : 'Perbesar preview'} onClick={onTheater}><PanelRightClose className="size-4" /></IconButton>
        <IconButton label={fullscreen ? 'Keluar layar penuh' : 'Preview layar penuh'} onClick={() => { void (document.fullscreenElement ? document.exitFullscreen() : stage.current!.requestFullscreen()).catch(() => { if (!theater) onTheater() }) }}>{fullscreen ? <Minimize className="size-4" /> : <Maximize className="size-4" />}</IconButton>
      </div>
    </section>
  )
}
