import { useState } from 'react'
import { Button, Modal, inputCls } from '../../components/ui'
import { useProject } from '../../store/project'
import { useApp } from '../../store/app'
import { errorText } from '../../lib/format'

const source = (): string => { const s = useProject.getState(); return JSON.stringify([s.project?.id,s.project?.title,s.project?.synopsis,s.clips.map(c => [c.id,c.story,c.narration])]) }

export function YoutubeDialog({open,onClose}: {open:boolean;onClose():void}) {
  const {project,updateEditor,flush} = useProject()
  const toast = useApp(s => s.toast)
  const [busy,setBusy] = useState(false)
  if (!project || project.aspectRatio !== '16:9') return null
  const data = project.editor.youtubeMetadata
  const generate = async (): Promise<void> => {
    setBusy(true)
    try {
      await flush()
      const before = source()
      const result = await window.api.story.youtubeMetadata(project.id)
      if (source() !== before) throw new Error('Naskah berubah selama pembuatan. Generate ulang agar deskripsi sesuai versi terbaru.')
      updateEditor({youtubeMetadata:result})
    } catch(e) { toast('error',errorText(e)) } finally {setBusy(false)}
  }
  const copy = async(text:string):Promise<void> => { try { await window.api.app.copyText(text); toast('success','Tersalin ke clipboard.') } catch(e) {toast('error',errorText(e))} }
  return <Modal open={open} onClose={onClose} title="Deskripsi dan tag YouTube" subtitle="Khusus video 16:9 · dibuat dari isi naskah" width={720}>
    <div className="flex flex-col gap-4 p-5">
      <p className="text-sm text-ink-2">Buat deskripsi singkat dan tag yang sesuai video menggunakan layanan AI naskah proyek. Hasil tersimpan di proyek dan terpisah dari MP4.</p>
      <Button variant="primary" className="self-start" loading={busy} onClick={() => void generate()}>{data ? 'Buat ulang deskripsi dan tag' : 'Buat deskripsi dan tag'}</Button>
      {data && <>
        <label className="flex flex-col gap-2 text-sm font-semibold">Deskripsi<textarea className={inputCls+' min-h-36 py-3 font-normal'} value={data.description} onChange={e => updateEditor({youtubeMetadata:{...data,description:e.target.value}})} /></label>
        <Button size="sm" className="self-start" onClick={() => void copy(data.description)}>Salin deskripsi</Button>
        <label className="flex flex-col gap-2 text-sm font-semibold">Tag (pisahkan dengan koma)<textarea className={inputCls+' min-h-20 py-3 font-normal'} value={data.tagsText ?? data.tags.join(', ')} onChange={e => updateEditor({youtubeMetadata:{...data,tagsText:e.target.value,tags:e.target.value.split(',').map(s=>s.trim()).filter(Boolean)}})} /></label>
        <Button size="sm" className="self-start" onClick={() => void copy(data.tags.join(', '))}>Salin tag</Button>
      </>}
      <p className="text-xs text-muted">Pemakaian mengikuti kuota/biaya model naskah. Periksa isi sebelum dipublikasikan.</p>
    </div>
  </Modal>
}
