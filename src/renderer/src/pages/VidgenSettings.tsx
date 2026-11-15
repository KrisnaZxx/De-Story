import { useEffect, useState } from 'react'
import type { AppSettings, VidgenAccount } from '@shared/types'
import { Button, Field, Switch, confirmDialog, inputCls } from '../components/ui'
import { errorText } from '../lib/format'

export function VidgenSettings({ settings, onSettings }: { settings: AppSettings; onSettings: (s: AppSettings) => void }) {
  const [accounts, setAccounts] = useState<VidgenAccount[]>([])
  const [editing, setEditing] = useState<string | null>(null)
  const [label, setLabel] = useState('')
  const [key, setKey] = useState('')
  const [busy, setBusy] = useState('')
  const [message, setMessage] = useState('')
  useEffect(() => { void window.api.vidgen.accounts().then(setAccounts).catch((e) => setMessage(errorText(e))) }, [])
  const run = async (name: string, fn: () => Promise<void>): Promise<void> => {
    setBusy(name); setMessage('')
    try { await fn() } catch (e) { setMessage(errorText(e)) } finally { setBusy('') }
  }
  const duplicate = (a: VidgenAccount): boolean => !!a.info?.user?.uid && accounts.some((b) => b.id !== a.id && b.info?.user?.uid === a.info?.user?.uid)
  const remove = async (a: VidgenAccount): Promise<void> => {
    if (await confirmDialog({ title: 'Hapus akun Vidgen dari De Story?', body: `API key tersimpan untuk ${a.label} dihapus dari laptop. Hasil video proyek tetap tersimpan.`, confirm: 'Hapus akun', danger: true })) {
      await run(a.id, async () => { setAccounts(await window.api.vidgen.remove(a.id)) })
    }
  }
  return <>
    <div><h1 className="font-display text-2xl font-bold">Vidgen multiakun</h1><p className="mt-2 text-base text-ink-2">Gambar memakai kredit berbayar. Video Omni 1.2 Flash memakai paket Unlimited aktif. Pilih model pada proyek.</p></div>
    <section className="flex flex-col gap-4 rounded-2xl border border-line bg-surface p-5">
      <h2 className="text-lg font-bold">{editing ? 'Ganti akun / API key' : 'Tambahkan akun'}</h2>
      <Field label="Nama akun"><input className={inputCls} value={label} disabled={!!busy} maxLength={80} onChange={(e) => setLabel(e.target.value)} placeholder="Akun utama" /></Field>
      <Field label={editing ? 'API key baru (kosongkan untuk mempertahankan key lama)' : 'API key Vidgen'}><input className={inputCls} type="password" autoComplete="off" spellCheck={false} value={key} disabled={!!busy} onChange={(e) => setKey(e.target.value)} placeholder="vg_…" /></Field>
      <p className="text-sm text-muted">Buat key di vidgen.id → Akun. Key disimpan terenkripsi di laptop dan dikirim hanya ke API Vidgen. Tidak perlu cookie atau Chrome for Testing.</p>
      <div className="flex gap-2"><Button variant="primary" loading={busy === 'save'} disabled={!!busy || !label.trim() || (!editing && !key.trim())} onClick={() => void run('save', async () => {
        const enabled = accounts.find((a) => a.id === editing)?.enabled ?? true
        setAccounts(await window.api.vidgen.save(editing, label, key, enabled)); setEditing(null); setLabel(''); setKey('')
      })}>{editing ? 'Simpan perubahan' : 'Tambahkan akun'}</Button>{editing && <Button disabled={!!busy} onClick={() => { setEditing(null); setLabel(''); setKey('') }}>Batal</Button>}</div>
    </section>
    <section className="flex flex-col gap-4 rounded-2xl border border-line bg-surface p-5">
      <Field label="Maksimal akun yang berjalan bersamaan"><select className={inputCls} value={settings.vidgenParallelAccounts ?? 0} disabled={!!busy} onChange={(e) => { const value = Number(e.target.value); void run('limit', async () => onSettings(await window.api.settings.set({ vidgenParallelAccounts: value }))) }}>
        <option value={0}>Semua akun aktif yang tersambung</option>{Array.from({ length: Math.max(accounts.length, settings.vidgenParallelAccounts ?? 0) }, (_, i) => i + 1).map((n) => <option key={n} value={n}>{n} akun</option>)}
      </select></Field>
      <p className="text-sm text-ink-2">Satu pekerjaan aktif per akun, beberapa akun berbeda berjalan bersamaan. Klip lainnya mengantre otomatis. Dua key dari akun Vidgen yang sama tetap dihitung satu akun. Pilihan dropdown dan sakelar langsung tersimpan.</p>
      <p className="text-sm text-muted">Model ini menghasilkan video 15 detik. Paket Unlimited tiap akun harus aktif; kuota harian dan batas server tetap berlaku. Menekan Periksa akun hanya membaca status dan tidak membuat video.</p>
      {!accounts.length && <p className="text-sm text-muted">Belum ada akun. Tambahkan API key saat sudah siap melakukan pengujian.</p>}
      {accounts.map((a) => <div key={a.id} className="flex flex-col gap-3 rounded-xl border border-line p-4">
        <div className="flex items-center justify-between gap-3"><div className="min-w-0"><p className="truncate font-semibold">{a.label}</p><p className="truncate text-sm text-muted">{a.info?.user?.email ?? (a.configured ? 'API key tersimpan' : 'Key perlu dimasukkan ulang')}</p></div><Switch label={`Aktifkan ${a.label}`} on={a.enabled} onChange={(enabled) => { if (!busy) void run(a.id, async () => setAccounts(await window.api.vidgen.save(a.id, a.label, '', enabled))) }} /></div>
        {a.info?.ok && <p className="text-sm text-ink-2">Kredit {a.info.user?.credits ?? '—'} · Unlimited {a.info.unlimited?.active ? 'aktif' : 'tidak aktif'} · sisa video harian {a.info.limits?.dailyVideosRemaining ?? '—'} · job server aktif {a.info.limits?.currentActiveJobs ?? '—'}</p>}
        {a.info && !a.info.ok && <p className="text-sm text-bad-ink">{a.info.error}</p>}
        {duplicate(a) && <p className="text-sm text-sun-ink">Akun ini sama dengan key lain di daftar; tidak menambah kapasitas paralel.</p>}
        <div className="flex flex-wrap gap-2"><Button size="sm" loading={busy === a.id} disabled={!!busy} onClick={() => void run(a.id, async () => setAccounts(await window.api.vidgen.check(a.id)))}>Periksa akun</Button><Button size="sm" disabled={!!busy} onClick={() => { setEditing(a.id); setLabel(a.label); setKey('') }}>Edit</Button><Button size="sm" variant="danger" disabled={!!busy} onClick={() => void remove(a)}>Hapus</Button></div>
      </div>)}
      <p className="text-sm text-muted">Hasil diunduh otomatis ke proyek. Membatalkan di De Story menghentikan penantian; pekerjaan yang sudah diterima Vidgen masih bisa selesai di dashboard mereka.</p>
    </section>
    {message && <p role="alert" className="rounded-xl bg-bad-soft p-4 text-sm text-bad-ink">{message}</p>}
  </>
}
