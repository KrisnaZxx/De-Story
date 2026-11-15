import { useEffect, useState } from 'react'
import type { FlowSessionsState } from '@shared/types'
import { Button, Switch, confirmDialog, cx, inputCls } from '../components/ui'
import { errorText } from '../lib/format'

const labels:Record<string,string> = {idle:'Belum diperiksa',login:'Selesaikan login',ready:'Sesi siap',needs_login:'Perlu login ulang',refreshing:'Memperbarui sesi',error:'Perlu diperiksa'}
export function FlowSessions({ready}: {ready:boolean}) {
  const [state,setState] = useState<FlowSessionsState|null>(null)
  const [busy,setBusy] = useState('')
  const [error,setError] = useState('')
  const [onlyProblems,setOnlyProblems] = useState(false)
  const load = async():Promise<void> => {try {setState(await window.api.flow2api.sessions());setError('')}catch(e){setError(errorText(e))}}
  useEffect(()=>{if(!ready)return;let alive=true; const refresh=async():Promise<void>=>{try {const next=await window.api.flow2api.sessions();if(alive){setState(next);setError('')}}catch(e){if(alive)setError(errorText(e))}};void refresh();const timer=setInterval(()=>void refresh(),5000);return()=>{alive=false;clearInterval(timer)}},[ready])
  const run = async(action:string,profileId?:string,extra:Record<string,unknown>={}):Promise<void> => {
    setBusy(profileId ?? action);setError('')
    try{setState(await window.api.flow2api.sessionAction({action,profileId,...extra}))}catch(e){setError(errorText(e))}finally{setBusy('')}
  }
  return <section className="flex flex-col gap-4 rounded-2xl border border-line bg-surface p-5">
    <div><h3 className="text-lg font-bold">Sesi login Flow</h3><p className="mt-1 text-sm text-ink-2">Login manual di Chrome biasa, tutup jendela profil setelah berhasil masuk Google Flow, lalu klik Selesai login dan simpan. Tidak menyimpan password Google.</p></div>
    {!ready && <p className="text-sm text-muted">Tunggu Flow2API internal siap untuk mengelola sesi.</p>}
    <div className="flex flex-wrap gap-2">
      <Button size="sm" disabled={!ready||!!busy} onClick={()=>void run('add')}>Tambah profil akun</Button>
      <Button size="sm" disabled={!ready||!!busy||state?.bulkRunning} onClick={()=>void run('attach-all')}>Tautkan semua akun tersimpan</Button>
      <Button size="sm" disabled={!ready||!!busy||state?.bulkRunning} onClick={()=>void run('renew-all')}>Perbarui semua sesi</Button>
      <Button size="sm" disabled={!ready||!!busy} onClick={()=>void load()}>Muat ulang status</Button>
    </div>
    {state && <>
      <div className="flex flex-wrap items-center gap-4 rounded-xl bg-sand p-3">
        <div className="flex items-center gap-2"><Switch label="Perbarui sesi otomatis" on={state.autoRefresh} onChange={v=>void run('config',undefined,{autoRefresh:v})}/><span className="text-sm">Perbarui sesi otomatis</span></div>
        <label className="flex items-center gap-2 text-sm">Interval<select className={cx(inputCls,'h-9 w-28')} value={state.intervalMinutes} disabled={!!busy} onChange={e=>void run('config',undefined,{intervalMinutes:Number(e.target.value)})}>{[10,15,20,30,45,60].map(n=><option key={n} value={n}>{n} menit</option>)}</select></label>
      </div>
      {state.bulkRunning && <p role="status" className="text-sm text-sun-ink">Memperbarui akun secara bertahap agar browser tidak membebani komputer. Akun sehat tetap dapat dipakai.</p>}
      <label className="flex items-center gap-2 text-sm"><input type="checkbox" checked={onlyProblems} onChange={e=>setOnlyProblems(e.target.checked)}/>Tampilkan hanya akun yang perlu tindakan</label>
      {state.accounts.filter(a=>!a.managed).map(a=><div key={a.tokenId} className="flex flex-wrap items-center justify-between gap-2 rounded-xl border border-line p-3"><span className="text-sm">{a.email} · belum mempunyai profil</span><Button size="sm" disabled={!!busy} onClick={()=>void run('add',undefined,{tokenId:a.tokenId})}>Tautkan profil</Button></div>)}
      {state.profiles.filter(p=>!onlyProblems||p.state!=='ready').map(p=><article key={p.id} className="flex flex-col gap-2 rounded-xl border border-line p-4">
        <div className="flex flex-wrap items-center justify-between gap-2"><strong className="break-all text-sm">{p.email||'Akun baru — belum login'}</strong><span className={cx('text-xs font-semibold',p.state==='ready'?'text-ok-ink':p.state==='needs_login'?'text-bad-ink':'text-ink-2')}>{labels[p.state]??p.state}</span></div>
        <p className="text-xs text-muted">Pembaruan terakhir: {p.lastSync?new Date(p.lastSync).toLocaleString('id-ID'):'belum ada'}</p>
        {p.error&&<p className="text-xs text-bad-ink">{p.error}</p>}
        <div className="flex flex-wrap gap-2">
          <Button size="sm" disabled={!!busy||p.busy} loading={busy===p.id} onClick={()=>void run('open',p.id)}>Buka profil / login</Button>
          {(p.browserOpen||p.state==='login')&&<Button size="sm" variant="primary" disabled={!!busy||p.busy} onClick={()=>void run('complete',p.id)}>Selesai login dan simpan</Button>}
          <Button size="sm" disabled={!!busy||p.busy||p.browserOpen||!p.tokenId||p.state==='refreshing'} onClick={()=>void run('renew',p.id)}>Perbarui sesi</Button>
          <Button size="sm" variant="danger" disabled={!!busy||p.busy} onClick={async()=>{if(await confirmDialog({title:'Lepas profil akun?',body:'Tautan profil dilepas dari daftar ini. Akun Flow dan file profil browser tetap disimpan pada komputer.',confirm:'Lepas profil',danger:true}))void run('remove',p.id)}}>Lepas profil</Button>
        </div>
      </article>)}
      {!state.profiles.length&&!state.accounts.length&&<p className="text-sm text-muted">Tambahkan profil akun, buka browser, login Google Flow, lalu klik Selesai login dan simpan.</p>}
    </>}
    {error&&<p role="alert" className="break-words text-sm text-bad-ink">{error}</p>}
    <p className="text-xs leading-relaxed text-muted">Login memakai Chrome/Chromium reguler yang terpasang di komputer. Tutup hanya jendela profil login setelah selesai, agar sesi dapat dibaca aplikasi. Profil tetap tersimpan; Google tetap dapat meminta login/verifikasi. Browser generasi dibersihkan setelah sekitar satu menit tidak aktif. Tidak perlu ekspor cookie berulang.</p>
  </section>
}
