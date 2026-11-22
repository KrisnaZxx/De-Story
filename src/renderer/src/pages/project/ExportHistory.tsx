import { Button, Modal, Progress } from '../../components/ui'
import { isRunning, useProject } from '../../store/project'
import { useApp } from '../../store/app'
import { errorText } from '../../lib/format'

const labels = {queued:'Mengantre',running:'Memproses',done:'Selesai',failed:'Gagal',canceled:'Dibatalkan'}
export function ExportHistory({open,onClose,onExport}: {open:boolean;onClose():void;onExport():void}) {
  const {jobs,project} = useProject()
  const toast = useApp(s=>s.toast)
  const exports = Object.values(jobs).filter(j=>j.kind==='export' && j.projectId===project?.id).sort((a,b)=>b.createdAt-a.createdAt)
  const exporting = exports.some(isRunning)
  const run = async(action:()=>Promise<unknown>):Promise<void>=>{try {await action()} catch(e){toast('error',errorText(e))}}
  return <Modal open={open} onClose={onClose} title="Riwayat ekspor" subtitle={project?.title} width={780}>
    <div className="flex flex-col gap-3 p-5">
      <p className="text-sm text-ink-2">Progres tetap berjalan saat dialog ditutup. Biarkan aplikasi aktif sampai hasil lokal atau unduhan VPS selesai.</p>
      <Button className="self-start" disabled={exporting} onClick={onExport}>Ekspor baru</Button>
      {exporting && <p className="text-sm text-muted">Tunggu ekspor aktif selesai atau batalkan sebelum mengekspor ulang.</p>}
      {!exports.length && <p className="rounded-xl bg-sand p-5 text-sm text-muted">Belum ada ekspor untuk proyek ini.</p>}
      {exports.map(j=><article key={j.id} className="flex flex-col gap-2 rounded-xl border border-line p-4">
        <div className="flex flex-wrap items-center justify-between gap-2"><strong className={j.status==='failed'?'text-bad-ink':j.status==='done'?'text-ok-ink':''}>{labels[j.status]}{isRunning(j)?` · ${Math.round(j.progress*100)}%`:''}</strong><time className="text-xs text-muted">{new Date(j.createdAt).toLocaleString('id-ID')}</time></div>
        {j.exportOptions && <p className="text-xs text-ink-2">{j.exportOptions.resolution===2160?'4K':j.exportOptions.resolution===1440?'2K (1440p)':`${j.exportOptions.resolution}p`} · {j.exportOptions.fps} FPS · {j.exportOptions.target==='ssh'?`VPS ${j.exportOptions.sshProfile}`:'Lokal'}</p>}
        {j.exportOptions?.target==='ssh' && (j.exportOptions.sshProfiles?.length??0)>1 && <p className="text-xs text-muted">VPS render: {j.exportOptions.sshProfiles!.join(', ')} · penggabungan di {j.exportOptions.sshProfile}</p>}
        <p className="break-words text-sm text-ink-2">{j.status==='failed'?j.error:j.message}</p>
        {isRunning(j) && <Progress value={j.progress} />}
        {j.status==='done' && j.resultAssetId && <p className="break-all font-mono text-xs text-muted">{j.resultAssetId}</p>}
        <div className="flex flex-wrap gap-2">
          {j.status==='done' && j.resultAssetId && <Button size="sm" onClick={()=>void run(()=>window.api.exporter.reveal(j.resultAssetId!))}>Buka folder hasil</Button>}
          {isRunning(j)?<Button size="sm" variant="danger" onClick={()=>void run(()=>window.api.generate.cancel(j.id))}>Batalkan ekspor</Button>:<Button size="sm" disabled={exporting} onClick={onExport}>Ekspor ulang</Button>}
        </div>
      </article>)}
    </div>
  </Modal>
}
