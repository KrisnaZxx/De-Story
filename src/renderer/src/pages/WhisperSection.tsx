import { useEffect, useState } from 'react'
import { AudioLines, Captions, Download, FolderOpen, Trash2 } from 'lucide-react'
import type { AppSettings, WhisperRemoteStatus, WhisperStatus } from '@shared/types'
import { Badge, Button, Field, Progress, Switch, confirmDialog, inputCls } from '../components/ui'
import { errorText } from '../lib/format'

const mb = (n: number): string => `${Math.round(n / 1_000_000).toLocaleString('id-ID')} MB`

/** Whisper can time captions locally or through the saved SSH connection. */
export function WhisperSection({ settings, onSettings }: { settings: AppSettings; onSettings: (s: AppSettings) => void }) {
  const [status, setStatus] = useState<WhisperStatus | null>(null)
  const speko = settings.captionEngine === 'speko'
  const remote = !speko && settings.whisperTarget === 'ssh'
  const [server, setServer] = useState<WhisperRemoteStatus | null>(null)
  const [remoteBusy, setRemoteBusy] = useState('')
  const [message, setMessage] = useState('')

  useEffect(() => {
    if (!remote) return
    let alive = true
    setServer(null)
    setMessage('')
    setRemoteBusy('check')
    void window.api.whisper.remoteStatus().then((s) => { if (alive) setServer(s) })
      .catch((e) => { if (alive) setMessage(errorText(e)) })
      .finally(() => { if (alive) setRemoteBusy('') })
    return () => { alive = false }
  }, [remote, settings.exportSshProfile, settings.whisperSshProfile])

  useEffect(() => {
    void window.api.whisper.status().then(setStatus)
    return window.api.on.whisper(setStatus)
  }, [])

  if (!status) return null
  const busy = status.state === 'downloading' || status.state === 'verifying'
  const saveTarget = async (target: AppSettings['whisperTarget']): Promise<void> => {
    setRemoteBusy('save')
    setMessage('')
    try { onSettings(await window.api.settings.set({ whisperTarget: target })) }
    catch (e) { setMessage(errorText(e)) }
    finally { setRemoteBusy('') }
  }
  const remoteAction = async (install: boolean): Promise<void> => {
    setRemoteBusy(install ? 'install' : 'check')
    setMessage('')
    try { setServer(await (install ? window.api.whisper.installRemote() : window.api.whisper.remoteStatus())) }
    catch (e) { setMessage(errorText(e)) }
    finally { setRemoteBusy('') }
  }

  const remove = async (): Promise<void> => {
    const ok = await confirmDialog({
      title: 'Hapus model Whisper?',
      body: `File model (${mb(status.total)}) dihapus dari komputer ini. Caption yang sudah disinkronkan tetap aman, dan model bisa diunduh lagi kapan saja.`,
      confirm: 'Hapus model',
      danger: true
    })
    if (ok) await window.api.whisper.remove()
  }

  return (
    <>
      <div className="mt-5 flex items-start gap-3">
        <span className="mt-1 text-ink">
          <Captions className="size-5" />
        </span>
        <div className="flex flex-col gap-1">
          <h2 className="font-display text-[22px] font-bold leading-tight">Caption akurat</h2>
          <p className="text-sm text-ink-2">Pilih pengenal suara untuk menyamakan waktu caption dengan audio Gemini atau audio yang sudah ada.</p>
        </div>
      </div>
      <section className="flex flex-col gap-4 rounded-2xl border border-line bg-surface px-5 py-[18px]">
        <Field label="Mesin sinkronisasi caption">
          <select aria-label="Mesin sinkronisasi caption" className={`${inputCls} h-11`} value={settings.captionEngine ?? 'whisper'} onChange={async (e) => { try { onSettings(await window.api.settings.set({ captionEngine: e.target.value as AppSettings['captionEngine'] })) } catch (e) { setMessage(errorText(e)) } }}>
            <option value="whisper">Whisper · lokal / VPS</option><option value="speko">Speko STT · cloud</option>
          </select>
        </Field>
        {speko ? <div className="flex flex-col gap-2 rounded-xl bg-info-soft p-4 text-sm text-info-ink">
          <p className="font-semibold">Gemini TTS → Speko STT → caption</p>
          <p>Isi API key dan pilih model STT di tab Speko. Model terpilih: {settings.spekoSttModel || 'belum dipilih'}. Audio panjang dibagi mengikuti batas unggah model; waktu hasil digabung kembali ke proyek.</p>
          <p>Tidak perlu mengunduh Whisper. Audio dikirim ke Speko dan ditagih per durasi setiap sinkronisasi. Suara Speko TTS dan ElevenLabs memakai waktu bawaan tanpa STT tambahan.</p>
        </div> : <>
        <Field label="Jalankan Whisper di">
          <select aria-label="Jalankan Whisper di" className={`${inputCls} h-11`} value={settings.whisperTarget ?? 'local'} disabled={!!remoteBusy} onChange={(e) => void saveTarget(e.target.value as AppSettings['whisperTarget'])}>
            <option value="local">Lokal — laptop ini</option>
            <option value="ssh">VPS melalui SSH</option>
          </select>
        </Field>
        {remote ? (
          <>
            <div className="flex flex-wrap items-center justify-between gap-2">
              <span className="font-semibold">Whisper Small · VPS {settings.whisperSshProfile || settings.exportSshProfile || 'belum dipilih'}</span>
              <Badge tone={server?.ready ? 'ok' : 'draft'}>{server?.ready ? 'Siap dipakai' : remoteBusy ? 'Memeriksa / menyiapkan' : 'Belum siap'}</Badge>
            </div>
            <p className="text-sm text-ink-2">Memakai koneksi di Pengaturan SSH. Audio dan naskah dikirim ke VPS, hasil waktu caption disimpan kembali di proyek. Audio sementara di VPS dibersihkan setelah proses selesai. Model lokal tidak perlu diunduh untuk mode ini.</p>
            <div className="flex flex-wrap gap-2">
              <Button loading={remoteBusy === 'check'} disabled={!!remoteBusy} onClick={() => void remoteAction(false)}>Periksa Whisper VPS</Button>
              <Button variant="accent" icon={<Download className="size-4" />} loading={remoteBusy === 'install'} disabled={!!remoteBusy} onClick={() => void remoteAction(true)}>Pasang Whisper di VPS</Button>
            </div>
            {remoteBusy === 'install' && <p className="text-sm text-sun-ink">Memasang whisper.cpp dan mengunduh model 488 MB langsung di VPS. Pemasangan pertama dapat memakan beberapa menit; biarkan aplikasi tetap terbuka.</p>}
            {server?.cpus != null && <p className="text-sm text-muted">{server.cpus} CPU · {server.freeGb} GB kosong</p>}
            <p className="text-sm text-muted">Laptop dan internet tetap aktif sampai sinkronisasi selesai. Whisper dan ekspor video memakai pilihan lokasi yang terpisah.</p>
          </>
        ) : (
        <>
        <div className="flex items-center gap-3">
          <span className="flex size-10 shrink-0 items-center justify-center rounded-[10px] border border-line-2 bg-accent-soft text-accent-ink">
            <AudioLines className="size-5" />
          </span>
          <div className="flex flex-1 flex-col">
            <span className="text-base font-bold">Whisper Small</span>
            <span className="text-[13px] text-ink-2">Pengenal suara multibahasa (termasuk Indonesia) lewat whisper.cpp · {mb(status.total)}</span>
          </div>
          {status.state === 'ready' ? (
            <Badge tone="ok">Siap dipakai</Badge>
          ) : busy ? (
            <Badge tone="warn">{status.state === 'verifying' ? 'Memeriksa file' : 'Mengunduh'}</Badge>
          ) : (
            <Badge tone="draft">Belum diunduh</Badge>
          )}
        </div>

        <p className="text-[13px] leading-relaxed text-ink-2">
          Dipakai untuk suara Gemini TTS, yang tidak mengirim waktu per kata. Suara ElevenLabs dan Speko TTS sudah membawa waktu kata. Setelah diunduh,
          Whisper berjalan tanpa internet dan tanpa biaya.
        </p>

        {busy && (
          <div className="flex flex-col gap-1.5">
            <Progress value={status.total ? status.received / status.total : 0} />
            <span className="font-mono text-xs text-ink-2">
              {mb(status.received)} dari {mb(status.total)}
              {status.state === 'verifying' ? ' · memeriksa keutuhan file…' : ''}
            </span>
          </div>
        )}
        {status.error && !busy && <p className="rounded-lg bg-bad-soft px-3 py-2 text-[13px] text-bad-ink">{status.error}</p>}

        <div className="flex flex-wrap gap-2">
          {status.state === 'ready' ? (
            <>
              <Button icon={<FolderOpen className="size-4" />} onClick={() => void window.api.whisper.openFolder()}>
                Buka folder
              </Button>
              <Button variant="danger" icon={<Trash2 className="size-4" />} onClick={() => void remove()}>
                Hapus model
              </Button>
            </>
          ) : busy ? (
            <Button disabled={status.state === 'verifying'} onClick={() => void window.api.whisper.cancel()}>
              Batal
            </Button>
          ) : (
            <Button variant="accent" icon={<Download className="size-4" />} onClick={() => void window.api.whisper.download()}>
              {status.received > 0 ? `Lanjutkan unduhan (${mb(status.total - status.received)} lagi)` : `Unduh model (${mb(status.total)})`}
            </Button>
          )}
        </div>
        </>
        )}
        </>}
        {message && <p role="alert" className="rounded-lg bg-bad-soft px-3 py-2 text-sm text-bad-ink">{message}</p>}

        <div className="flex items-center gap-3 border-t border-dashed border-line pt-4">
          <div className="flex-1">
            <p className="text-sm font-semibold">Sinkronkan otomatis</p>
            <p className="text-[13px] text-muted">Setiap kali suara narasi Gemini dibuat, caption langsung disamakan dengan suaranya.</p>
          </div>
          <Switch on={settings.whisperAuto} onChange={async (v) => {
            try { onSettings(await window.api.settings.set({ whisperAuto: v })) }
            catch (e) { setMessage(errorText(e)) }
          }} label="Sinkronkan caption otomatis" />
        </div>
      </section>
    </>
  )
}
