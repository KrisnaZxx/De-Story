# Pengembangan Bara Studio

[Kembali ke README](../README.md)

## Build dari source

Prasyarat: Node.js 22+, npm, Rust stable, Git. Build aplikasi Windows x64 memerlukan C++ Build Tools dan WebView2.

```bash
git clone https://github.com/KrisnaZxx/De-Story.git
cd De-Story
```

```bash
npm ci
npm run typecheck
npm run build
```

Source Flow2API yang disesuaikan ada di `flow2api-src`. Perintah build menyiapkan Python dan Chromium untuk Windows.

```powershell
npm run tauri:build
```

Hasil paket ada di `src-tauri/target/release/bundle`, kecuali `CARGO_TARGET_DIR` diubah.

## Pemeriksaan

```bash
node scripts/editor-checks.mjs
node scripts/vidgen-ui-checks.mjs
node scripts/speech-ui-checks.mjs
cargo test --lib --locked --manifest-path src-tauri/Cargo.toml -- --test-threads=1
```

Tes Python memerlukan dependensi Flow2API. Setelah staging, gunakan interpreter bawaan:

```powershell
# Windows
.\src-tauri\flow2api\python\python.exe scripts/flow2api/sessions_test.py
.\src-tauri\flow2api\python\python.exe scripts/flow2api/concurrency_test.py
```

## Membuat rilis

1. Samakan versi di `package.json`, `package-lock.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, `src-tauri/tauri.conf.json`, dan teks versi di pengaturan aplikasi.
2. Jalankan pemeriksaan dan push source.
3. Buka **Actions → Rilis Windows → Run workflow**. Alternatif: push tag versi `v…` untuk memulai workflow.
4. Tunggu build Windows x64 selesai. Workflow membuat draft dengan installer EXE dan MSI.
5. Periksa installer, status pengujian, dan catatan rilis sebelum menerbitkannya. Tandai pre-release jika masih untuk pengujian.
6. Pastikan rilis terbaru dapat dibuka lewat tautan unduh pada README.

Push ke `main` untuk perubahan dokumentasi tidak menjalankan build installer; workflow saat ini berjalan melalui tag atau pemicu manual. Jangan memindahkan tag rilis lama untuk perubahan README.

Publikasi draft dapat memicu build lagi saat tag dibuat. Jika commit yang sama sudah berhasil dibangun, batalkan job duplikat agar aset/checksum rilis tidak berubah tanpa perlu.

## Menyiapkan GitHub dan kredit

Upload source, konfigurasi build, dokumentasi, dan lisensi komponen. .gitignore mengecualikan instalasi lokal, runtime staging, binary, .toolchain, database, log, dan file key. Jangan upload folder data, cookie, private key, atau API key. File yang pernah dilacak masih perlu diperiksa; .gitignore tidak menghapus rahasia dari riwayat.

Branding dan metadata pengembang menggunakan **KrisnaZxx**. Hak cipta/lisensi font, ikon, Whisper, Flow2API, dan komponen lain tetap dipertahankan di resources/licenses, flow2api-src/LICENSE, serta notices runtime. Nama lama khusus migrasi folder/keyring diperlukan agar data lama tetap terbaca; itu bukan kredit antarmuka.

## Pemeriksaan regresi tambahan

- `python scripts/flow2api/concurrency_test.py`: akun habis kuota, perpindahan akun, antrean saat akun sehat sibuk, dan pencatatan kredit. Memerlukan dependensi Python Flow2API.
- `python scripts/ssh_worker_test.py`: batas direktori, PID lama, dan pelestarian hasil ekspor.
- `python scripts/ssh_worker_linux_test.py`: FFmpeg Linux nyata dengan HOME sementara dan video sintetis; tidak menyentuh proyek/hasil pengguna.
- `node scripts/speech-ui-checks.mjs`: kesiapan layanan suara dan kontras tema gelap/terang.

Nama produk v1.4.0 adalah Bara Studio. Identifier aplikasi, nama executable internal, folder data De Story, dan nama layanan keyring dipertahankan untuk kompatibilitas proyek serta kredensial.

- `node scripts/speko-language-checks.mjs`: model khusus Inggris/Arab ditolak untuk Indonesia, dukungan belum terverifikasi diblokir, serta bahasa regional dinormalisasi. Katalog bahasa dipakai bersama oleh antarmuka dan pemeriksaan native.

## Render multi-VPS (v1.5.0)

`ExportOptions.sshProfile` adalah VPS penggabungan. `sshProfiles` memuat profil render tambahan; native menyertakan profil utama dan menghapus duplikasi tanpa membedakan huruf besar/kecil. Klip dibagi menjadi kelompok berurutan berdasarkan durasi beserta overlap menggunakan rencana timeline yang sama. Setiap node mengunggah satu batch worker.

Worker mode segmen menghasilkan metadata `kind: segments`, daftar nama/ukuran/checksum. Mulai v1.5.1, worker sumber membuka port HTTPS privat InstaBox secara sementara; VPS utama mengambil bagian dan memeriksa ukuran/checksum. Laptop hanya meneruskan tiket kecil melalui SSH, tanpa menyalin kredensial SSH ke VPS. Bagian pada VPS utama sendiri disalin di server yang sama. Helper `ssh_transfer.py` hanya menyajikan nama segmen terdaftar, tanpa listing direktori. Port ditutup setelah transfer atau batas waktu satu jam. Ekspor satu VPS tidak memerlukan instabox-port.

Pemeriksaan tambahan:

- Unit native `distributed_render_balances_duration_and_preserves_every_clip_once`: pembagian, batas jumlah node, deduplikasi profil, dan penolakan nama profil tidak aman.
- `python scripts/ssh_worker_test.py`: validasi nama segmen dan pelestarian batch terverifikasi.
- `python scripts/ssh_worker_linux_test.py`: batch sukses, checksum, hapus hasil, segmen hilang, dan pembatalan tanpa hasil parsial.
- Uji native opsional `media::exporter::tests::ssh_distributed_render_round_trip_and_result_management` memakai `DE_STORY_SSH_PROFILE` dan `DE_STORY_SSH_SECOND_PROFILE`. Dua node merender bersamaan, hasil diverifikasi/dipindahkan, lalu digabung dengan caption dan audio. Hanya video sintetis, tanpa layanan AI berbayar. Kedua profil harus sudah siap render; jika profil kedua tidak disetel, memakai koneksi utama untuk menguji dua job pada satu server.

Alur belum mendukung melanjutkan penggabungan dari checkpoint setelah aplikasi ditutup. Pengujian dua alias pada satu server memvalidasi transfer dan isolasi job, tetapi bukan tolok ukur kecepatan dua server fisik.

### Regresi memori narasi panjang

`narration_graph` menyusun setiap suara dengan resample, padding/trim sesuai durasi klip, lalu concat. Klip tanpa narasi diberi sumber hening dengan durasi yang sama. Uji audio terpisah menunjukkan graph lama dan baru sama-sama ringan pada fixture; jangan menganggap audio sebagai penyebab OOM tanpa pengukuran.

Uji opsional native `media::exporter::tests::write_narration_memory_fixture` menulis graph sintetis ke `DE_STORY_RENDER_FIXTURE_DIR` (folder `render-memory-*` di `.toolchain`). Salin `graphs.json` dan jalankan `python3 scripts/render_memory_test.py /path/to/graphs.json` pada Linux dengan FFmpeg. Pengujian membandingkan graph delay lama dengan concat baru untuk 60 klip / 10 menit di bawah batas address space 1536 MiB, lalu mengukur ketepatan suara dan jeda pada hasil PCM. Tidak memerlukan layanan AI.

Worker menyaring baris progres dari ekor diagnostik dan mencatat exit code serta tahap gagal. Tes Linux juga memeriksa encoder yang menerima SIGKILL, agar pesan galat tidak lagi hanya menampilkan frame/fps/progress.

### Penggabungan video dengan batas input

`merge_video_parts` memakai graph transisi yang sama, tetapi menggabungkan maksimal empat bagian per tahap. Durasi bagian adalah jumlah own ditambah outgoing overlap terakhir; start global dan transisi terakhir diteruskan ke rencana induk. Ini menjaga transisi di batas node. Bagian dirangkai di tiap node sebelum transfer. Untuk jumlah node besar, render satu VPS, dan lokal, batas empat input tetap diterapkan secara bertahap. Ada encoding tambahan pada bagian; CRF 17 menjaga mutu intermediate, tetapi prosesnya tidak lossless.

Uji native opsional `media::exporter::tests::ssh_grouped_2k_memory_regression` merender 20 klip sintetis 2K dengan tiga tingkat penggabungan di bawah batas address space 3 GiB. Batas ini hanya pada worker fixture, bukan batas aplikasi. `python scripts/ssh_transfer_test.py` memeriksa nama/hash/ukuran, larangan overwrite, sumber yang dipertahankan saat checksum gagal, dan kebutuhan HTTPS.
