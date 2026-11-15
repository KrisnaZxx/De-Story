import { useState } from 'react'
import { Download, FolderOpen, RefreshCw, Server, Trash2 } from 'lucide-react'
import type { AppSettings, SshConnection, SshResult, SshResults, SshStatus } from '@shared/types'
import { Button, Field, confirmDialog, cx, inputCls } from '../components/ui'
import { errorText } from '../lib/format'

export function SshSettings({ settings, onSettings }: { settings: AppSettings; onSettings: (settings: AppSettings) => void }) {
  const connections = settings.sshConnections?.length ? settings.sshConnections : settings.sshConnection ? [settings.sshConnection] : []
  const [selected, setSelected] = useState(settings.exportSshProfile ?? '')
  const [form, setForm] = useState<SshConnection>(connections.find(c => c.profile === settings.exportSshProfile) ?? connections[0] ?? {
    profile: settings.exportSshProfile ?? 'instabox-render', host: '', port: 22, username: '', identityFile: ''
  })
  const [password, setPassword] = useState('')
  const [status, setStatus] = useState<SshStatus | null>(null)
  const [listing, setListing] = useState<SshResults | null>(null)
  const [busy, setBusy] = useState<string | null>(null)
  const [message, setMessage] = useState('')
  const [failed, setFailed] = useState(false)
  const [deleteAfter, setDeleteAfter] = useState(false)
  const [downloaded, setDownloaded] = useState<string | null>(null)
  const savedConnection = connections.find(c => c.profile === selected)
  const saved = !password && !!savedConnection && (['profile', 'host', 'port', 'username', 'identityFile', 'authMethod'] as const).every((key) => form[key] === savedConnection[key])
  const profile = selected
  const set = (patch: Partial<SshConnection>): void => { setForm({ ...form, ...patch }); setStatus(null) }
  const refresh = async (): Promise<void> => setListing(await window.api.ssh.results(profile))
  const run = async (name: string, action: () => Promise<void>): Promise<void> => {
    setBusy(name)
    setMessage('')
    setFailed(false)
    try { await action() }
    catch (e) { setFailed(true); setMessage(errorText(e)) }
    finally { setBusy(null) }
  }
  const remove = async (result: SshResult): Promise<void> => {
    const confirmed = await confirmDialog({
      title: 'Hapus hasil video di VPS?',
      body: `“${result.filename}” akan dihapus permanen dari VPS. Pastikan sudah mengunduhnya jika masih diperlukan.`,
      confirm: 'Hapus dari VPS', danger: true
    })
    if (confirmed) await run(`delete:${result.id}`, async () => {
      await window.api.ssh.delete(profile, result.id)
      await refresh()
      setMessage('Hasil di VPS sudah dihapus. Informasi storage diperbarui.')
    })
  }

  return (
    <>
      <div>
        <h1 className="flex items-center gap-3 font-display text-[34px] font-bold"><Server className="size-7" /> Setting SSH</h1>
        <p className="mt-2 text-base text-ink-2">Koneksi VPS untuk render FFmpeg dan mengelola hasil video.</p>
      </div>
      <section className="flex flex-col gap-4 rounded-2xl border border-line bg-surface p-6">
        <h2 className="text-lg font-bold">Profil VPS</h2>
        <div className="flex flex-wrap items-end gap-2">
          <label className="min-w-0 flex-1 text-sm">Pilih VPS<select className={inputCls + ' mt-1'} value={selected} disabled={!!busy} onChange={ev => { const c = connections.find(c => c.profile === ev.target.value); setSelected(ev.target.value); setPassword(''); if(c) setForm(c); setListing(null); setStatus(null) }}><option value="">Profil baru</option>{connections.map(c => <option key={c.profile} value={c.profile}>{c.profile} · {c.host}</option>)}</select></label>
          <Button disabled={!!busy} onClick={() => { setSelected(''); setPassword(''); setForm({ profile: '', host: '', username: '', port: 22, identityFile: '' }); setListing(null); setStatus(null) }}>Tambah VPS</Button>
          <Button variant="danger" disabled={!!busy || !selected} onClick={() => void run('remove', async () => { if (!await confirmDialog({ title: 'Hapus profil SSH?', body: 'Hanya profil koneksi di aplikasi yang dihapus. File dan video di VPS tetap tersedia.', confirm: 'Hapus profil', danger: true })) return; const next = await window.api.ssh.remove(selected); onSettings(next); setSelected(''); setPassword(''); setForm({profile:'',host:'',port:22,username:'',identityFile:''}); setListing(null); setStatus(null) })}>Hapus profil</Button>
        </div>
        <div className="grid gap-3 sm:grid-cols-2">
          <Field label="VPS bawaan ekspor"><select className={inputCls} value={settings.exportSshProfile} disabled={!!busy} onChange={ev => void run('default', async () => onSettings(await window.api.settings.set({ exportSshProfile: ev.target.value })))}><option value="">Pilih VPS</option>{connections.map(c => <option key={c.profile}>{c.profile}</option>)}</select></Field>
          <Field label="VPS untuk Whisper"><select className={inputCls} value={settings.whisperSshProfile ?? ''} disabled={!!busy} onChange={ev => void run('default', async () => onSettings(await window.api.settings.set({ whisperSshProfile: ev.target.value })))}><option value="">Ikuti VPS ekspor</option>{connections.map(c => <option key={c.profile}>{c.profile}</option>)}</select></Field>
        </div>
        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
          <Field label="Nama profil SSH"><input className={inputCls} value={form.profile} disabled={!!busy || !!selected} onChange={(e) => set({ profile: e.target.value })} placeholder="instabox-render" /></Field>
          <Field label="Host / alamat server"><input className={inputCls} value={form.host} disabled={!!busy} onChange={(e) => set({ host: e.target.value })} placeholder="ssh.instabox.sh" /></Field>
          <Field label="Username SSH"><input className={inputCls} value={form.username} disabled={!!busy} onChange={(e) => set({ username: e.target.value })} /></Field>
          <Field label="Port SSH"><input type="number" min={1} max={65535} className={inputCls} value={form.port} disabled={!!busy} onChange={(e) => set({ port: Number(e.target.value) })} /></Field>
        </div>
        <Field label="Metode login SSH"><select className={inputCls} value={form.authMethod ?? 'key'} disabled={!!busy} onChange={e=>set({authMethod:e.target.value as 'key'|'password'})}><option value="key">SSH key / agent</option><option value="password">Password akun VPS</option></select></Field>
        {form.authMethod === 'password' ? <Field label={form.passwordConfigured ? 'Password baru (kosongkan untuk mempertahankan sandi tersimpan)' : 'Password akun VPS'}><input type="password" autoComplete="new-password" className={inputCls} value={password} disabled={!!busy} onChange={e=>setPassword(e.target.value)} /><p className="text-xs text-muted">Sandi tersimpan terenkripsi pada komputer ini. Autentikasi SSH tetap memeriksa kunci host. OTP/verifikasi tambahan dan password sudo tidak diisi otomatis.</p></Field> : <Field label="File private key di laptop (opsional bila memakai SSH agent)">
          <div className="flex gap-2">
            <input className={cx(inputCls, 'min-w-0 flex-1')} value={form.identityFile} disabled={!!busy} onChange={(e) => set({ identityFile: e.target.value })} placeholder="~/.ssh/id_ed25519" />
            <Button disabled={!!busy} onClick={async () => { const path = await window.api.ssh.pickKey(); if (path) set({ identityFile: path }) }}>Pilih file</Button>
          </div>
        </Field>}
        <p className="text-sm text-ink-2">File private key dipakai oleh SSH di laptop. Aset media dan pengaturan render yang dikirim ke VPS.</p>
        <div className="flex flex-wrap gap-2">
          <Button variant="primary" loading={busy === 'save'} disabled={!!busy || saved} onClick={() => void run('save', async () => {
            const clean = { ...form, profile: form.profile.trim(), host: form.host.trim(), username: form.username.trim(), identityFile: form.identityFile.trim() }
            const next = await window.api.ssh.save({...clean,authMethod:clean.authMethod ?? 'key'}, password || undefined)
            onSettings(next)
            setForm(next.sshConnections?.find(c=>c.profile===clean.profile) ?? clean)
            setPassword('')
            setSelected(clean.profile)
            setListing(null)
            setMessage('Koneksi SSH tersimpan. Periksa koneksi atau pasang kebutuhan VPS.')
          })}>Simpan SSH</Button>
          <Button loading={busy === 'check'} disabled={!!busy || !saved} onClick={() => void run('check', async () => {
            const server = await window.api.exporter.testSsh(profile)
            setStatus(server)
            setMessage(server.ffmpeg && server.worker ? 'VPS siap untuk ekspor.' : 'SSH terhubung. Klik Pasang kebutuhan VPS untuk menyiapkan render.')
          })}>Periksa SSH</Button>
          <Button loading={busy === 'install'} disabled={!!busy || !saved} onClick={() => void run('install', async () => {
            setStatus(await window.api.ssh.install(profile))
            await refresh()
            setMessage('FFmpeg, dukungan caption, dan worker render sudah siap.')
          })}>Pasang kebutuhan VPS</Button>
        </div>
        <p className="text-sm text-muted">Pemasangan otomatis mendukung Debian/Ubuntu dengan akses sudo tanpa prompt. Kunci host SSH harus sudah dipercaya di terminal. Koneksi dapat diganti setelah ekspor selesai.</p>
        {status && <p className="rounded-xl bg-sand p-3 text-sm">{status.cpus} CPU · {status.freeGb} GB kosong dari {status.totalGb} GB · {status.ffmpeg ? 'FFmpeg terpasang' : 'FFmpeg belum terpasang'}</p>}
      </section>

      <section className="flex flex-col gap-4 rounded-2xl border border-line bg-surface p-6">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <h2 className="text-lg font-bold">Hasil video di VPS</h2>
          <Button icon={<RefreshCw className="size-4" />} loading={busy === 'results'} disabled={!!busy || !saved} onClick={() => void run('results', refresh)}>Cek semua hasil video</Button>
        </div>
        {listing && <>
          <div className="rounded-xl bg-sand p-4">
            <p className="font-semibold">Sisa storage: {listing.storage.freeGb} GB / {listing.storage.totalGb} GB</p>
            <p className="mt-1 text-sm text-ink-2">Terpakai {listing.storage.usedGb} GB pada disk penyimpanan VPS.</p>
            <div className="mt-3 h-2 overflow-hidden rounded-full bg-line"><div className="h-full bg-accent" style={{ width: `${Math.min(100, listing.storage.usedGb / listing.storage.totalGb * 100)}%` }} /></div>
          </div>
          <label className="flex items-center gap-3 text-sm"><input type="checkbox" checked={deleteAfter} disabled={!!busy} onChange={(e) => setDeleteAfter(e.target.checked)} /> Hapus video di VPS setelah download berhasil diverifikasi</label>
          {listing.results.length === 0 && <p className="text-sm text-muted">Belum ada hasil ekspor yang tersimpan di VPS ini.</p>}
          {listing.results.map((result) => <div key={result.id} className="flex flex-col gap-3 rounded-xl border border-line p-4">
            <div className="min-w-0">
              <p className="break-words font-semibold">{result.title || result.filename}</p>
              <p className="mt-1 break-all text-sm text-ink-2">{result.filename} · {(result.bytes / 1024 / 1024).toFixed(1)} MB · {new Date(result.createdAt * 1000).toLocaleString('id-ID')}</p>
              <p className="mt-1 text-sm text-muted">{result.state === 'completed' ? 'Siap diunduh' : result.state === 'running' ? 'Sedang dirender' : result.state === 'incomplete' ? 'Unggahan belum selesai' : 'Render gagal'}</p>
            </div>
            <div className="flex flex-wrap gap-2">
              <Button icon={<Download className="size-4" />} loading={busy === `download:${result.id}`} disabled={!!busy || result.state !== 'completed'} onClick={() => void run(`download:${result.id}`, async () => {
                const path = await window.api.ssh.download(profile, result.id, deleteAfter)
                setDownloaded(path)
                await refresh()
                setMessage(`Download berhasil dan checksum cocok: ${path}`)
              })}>Download melalui SSH</Button>
              <Button variant="danger" icon={<Trash2 className="size-4" />} disabled={!!busy || result.state === 'running'} onClick={() => void remove(result)}>Hapus dari VPS</Button>
            </div>
          </div>)}
        </>}
      </section>
      {message && <p role="status" className={cx('break-words rounded-xl p-4 text-sm', failed ? 'bg-bad-soft text-bad-ink' : 'bg-sand text-ink-2')}>{message}</p>}
      {downloaded && <Button className="self-start" icon={<FolderOpen className="size-4" />} onClick={() => void window.api.exporter.reveal(downloaded)}>Buka folder hasil download</Button>}
    </>
  )
}
