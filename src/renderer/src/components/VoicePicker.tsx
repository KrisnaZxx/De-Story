import { useEffect, useRef, useState } from 'react'
import { Play, Square } from 'lucide-react'
import type { TtsProvider, VoiceOption } from '@shared/types'
import { Select } from './Select'
import { cx } from './ui'
import { errorText } from '../lib/format'
import { geminiPreview } from '../lib/voicePreviews'

/** One sample plays at a time, from the list rows or the button beside the picker. */
function usePreviewPlayer(): { playing: string | null; stop: () => void; toggle: (id: string, url: string) => void } {
  const audio = useRef<HTMLAudioElement | null>(null)
  const [playing, setPlaying] = useState<string | null>(null)
  useEffect(() => () => audio.current?.pause(), [])
  return {
    playing,
    stop: () => { audio.current?.pause(); setPlaying(null) },
    toggle: (id, url) => {
      audio.current?.pause()
      if (playing === id) return setPlaying(null)
      const a = new Audio(url)
      a.onended = () => { if (audio.current === a) setPlaying(null) }
      audio.current = a
      setPlaying(id)
      void a.play().catch(() => { if (audio.current === a) setPlaying(null) })
    }
  }
}

export function VoicePicker({ provider, value, language = 'id', onChange }: { provider: TtsProvider; value: string; language?: string; onChange: (v: string) => void }) {
  const [voices, setVoices] = useState<VoiceOption[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const player = usePreviewPlayer()

  useEffect(() => {
    let alive = true
    player.stop()
    setVoices(null)
    setError(null)
    window.api.settings
      .voices(provider, language)
      .then((list) => {
        if (!alive) return
        // Gemini samples ship with the app; ElevenLabs sends its own preview links.
        const v = provider === 'gemini' ? list.map((x) => ({ ...x, previewUrl: x.previewUrl ?? geminiPreview(x.id, x.name) })) : list
        setVoices(v)
        if (provider !== 'speko' && !v.some((x) => x.id === value) && v[0]) onChange(v[0].id)
      })
      .catch((e) => { if (alive) setError(errorText(e)) })
    return () => { alive = false }
  }, [provider, language])

  const urlOf = (id: string): string | null => voices?.find((v) => v.id === id)?.previewUrl ?? null
  const current = voices?.find((v) => v.id === value)

  const playButton = (id: string, size: 'row' | 'field') => {
    const url = urlOf(id)
    if (!url) return null
    const on = player.playing === id
    return (
      <button
        type="button"
        aria-label={on ? 'Hentikan contoh suara' : 'Dengarkan contoh suara'}
        title={on ? 'Hentikan' : 'Dengarkan contoh'}
        onClick={() => player.toggle(id, url)}
        className={cx(
          'flex shrink-0 items-center justify-center rounded-full border transition-colors',
          size === 'row' ? 'size-7' : 'size-12 rounded-xl',
          on ? 'border-accent bg-accent text-white' : 'border-line-2 bg-surface text-ink hover:border-line-3'
        )}
      >
        {on ? <Square className="size-3.5" fill="currentColor" /> : <Play className="ml-px size-3.5" fill="currentColor" />}
      </button>
    )
  }

  if (error) return <p className="flex h-12 flex-1 items-center rounded-xl bg-bad-soft px-3.5 text-[13px] text-bad-ink">{error}</p>
  return (
    <>
      <Select
        label="Suara narator"
        size="lg"
        value={provider === 'speko' && !value ? 'auto' : value}
        onChange={onChange}
        disabled={!voices}
        className="min-w-[180px] flex-1"
        menuWidth={380}
        renderAction={(o) => playButton(o.value, 'row')}
        options={
          voices
            ? [...(provider === 'speko' && value && !voices.some(v => v.id === value) ? [{id:value,name:`Suara tersimpan · ${value}`,description:'Pilihan sebelumnya'}] : []), ...voices].map((v) =>
                // Gemini tones are short; ElevenLabs labels are longer, so they go on a second line.
                provider === 'gemini' || provider === 'speko' ? { value: v.id, label: v.name, hint: v.description || undefined } : { value: v.id, label: v.name, note: v.description || undefined }
              )
            : [{ value, label: 'Memuat daftar suara…' }]
        }
      />
      {current && playButton(current.id, 'field')}
      {provider === 'speko' && voices && <span className="max-w-[240px] text-xs text-muted">{voices.some(v => v.previewUrl) ? 'Klik ▶ untuk contoh suara. Memilih suara tidak memakai saldo.' : 'Model ini tidak menyediakan audio contoh siap putar. Memilih suara tidak memakai saldo.'}</span>}
    </>
  )
}

