import { useState } from 'react'
import { Button, Modal } from '../../components/ui'
import { useProject } from '../../store/project'
import { useApp } from '../../store/app'
import { isRunning } from '../../store/project'
import { errorText } from '../../lib/format'

export function ThumbnailDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const { project, clips, assets, jobs, flush, upsertJob } = useProject()
  const toast = useApp((s) => s.toast)
  const [starting, setStarting] = useState(false)
  const [downloading, setDownloading] = useState(false)
  if (!project || project.aspectRatio !== '16:9') return null
  const thumbnail = Object.values(assets).filter((a) => a.kind === 'thumbnail').sort((a, b) => b.createdAt - a.createdAt)[0]
  const job = Object.values(jobs).filter((j) => j.kind === 'thumbnail').sort((a, b) => b.createdAt - a.createdAt)[0]
  const running = isRunning(job)
  const generate = async (): Promise<void> => {
    setStarting(true)
    try { await flush(); upsertJob(await window.api.generate.thumbnail(project.id)) } catch (e) { toast('error', errorText(e)) } finally { setStarting(false) }
  }
  const download = async (): Promise<void> => {
    if (!thumbnail) return
    setDownloading(true)
    try { const path = await window.api.assets.downloadThumbnail(thumbnail.id); toast('success', 'Thumbnail PNG tersimpan.', { label: 'Buka folder', run: () => void window.api.exporter.reveal(path) }) } catch (e) { toast('error', errorText(e)) } finally { setDownloading(false) }
  }
  return <Modal open={open} onClose={onClose} width={820} title="Thumbnail YouTube otomatis">
    <div className="flex flex-col gap-4 p-6">
      <p className="text-sm text-ink-2">AI memilih isi cerita dan teks singkat, lalu model gambar proyek membuat thumbnail 16:9 beserta teksnya. Thumbnail tersimpan di proyek dan terpisah dari ekspor video.</p>
      <div className="aspect-video min-w-0 overflow-hidden rounded-xl border border-line bg-paper">
        {thumbnail ? <img className="h-full w-full object-contain" src={thumbnail.url} alt="Thumbnail YouTube" /> : <div className="flex h-full items-center justify-center p-5 text-center text-muted">{running ? job?.message ?? 'Sedang membuat thumbnail…' : 'Belum ada thumbnail'}</div>}
      </div>
      {running && <p role="status" className="text-sm text-sun-ink">{job?.message ?? 'Menunggu hasil gambar'} · {Math.round((job?.progress ?? 0) * 100)}%</p>}
      {job?.status === 'failed' && <p role="alert" className="text-sm text-bad-ink">{job.error}</p>}
      <div className="flex flex-wrap gap-2">
        <Button variant="primary" loading={starting || running} disabled={!clips.length || starting || running} onClick={() => void generate()}>{thumbnail ? 'Buat ulang thumbnail' : 'Buat thumbnail otomatis'}</Button>
        {thumbnail && <Button loading={downloading} disabled={downloading} onClick={() => void download()}>Unduh PNG 1280 × 720</Button>}
        {running && job && <Button onClick={() => void window.api.generate.cancel(job.id).catch((e) => toast('error', errorText(e)))}>Batalkan proses</Button>}
        <Button onClick={onClose}>Tutup</Button>
      </div>
      <p className="text-xs text-muted">Generasi memakai kuota layanan gambar proyek. Hasil lama tetap disimpan saat membuat ulang. Ejaan teks dari AI bisa kurang tepat.</p>
    </div>
  </Modal>
}
