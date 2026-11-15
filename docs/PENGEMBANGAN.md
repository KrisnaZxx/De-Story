# Pengembangan De Story

[Kembali ke README](../README.md)

## Build dari source

Prasyarat: Node.js 22+, npm, Rust stable, Git. Windows memerlukan C++ Build Tools dan WebView2; Linux mengikuti [prasyarat Tauri](https://v2.tauri.app/start/prerequisites/#linux).

```bash
git clone https://github.com/KrisnaZxx/De-Story.git
cd De-Story
```

```bash
npm ci
npm run typecheck
npm run build
```

Source Flow2API yang disesuaikan ada di flow2api-src. Staging mengunduh Python/Chromium sesuai OS build dan cukup besar. Jangan memakai runtime Windows untuk paket Linux.

Windows:

```powershell
npm run tauri:build
```

Linux untuk developer/build server:

```bash
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev libdbus-1-dev cmake patchelf ffmpeg openssh-client libsecret-1-dev
npm run tauri:build:linux
```

macOS memerlukan Xcode Command Line Tools dan CMake. FFmpeg harus tersedia di `PATH` untuk pemakaian aplikasi. Build Whisper terlebih dahulu:

```bash
bash scripts/build-whisper-unix.sh
npm run tauri:build
```

Whisper UNIX dikompilasi dari commit yang dipatok di scripts/build-whisper-unix.sh, tanpa optimasi CPU khusus mesin build. End user cukup memakai installer GUI. Untuk pengembangan: build Whisper pada UNIX, stage Flow2API, lalu npm run tauri:dev.

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

```bash
# Linux / macOS
./src-tauri/flow2api/python/bin/python3 scripts/flow2api/sessions_test.py
./src-tauri/flow2api/python/bin/python3 scripts/flow2api/concurrency_test.py
```

## Membuat rilis

1. Samakan versi di `package.json`, `package-lock.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, `src-tauri/tauri.conf.json`, dan teks versi di pengaturan aplikasi.
2. Jalankan pemeriksaan dan push source.
3. Buka **Actions → Release & Multi-Platform Build → Run workflow**. Alternatif: push tag versi `v…` untuk memulai workflow.
4. Tunggu build Windows x64, Linux x64, dan macOS ARM64 selesai. Workflow menyiapkan runtime per OS dan membuat draft release.
5. Periksa installer, status pengujian, dan catatan rilis sebelum menerbitkannya. Tandai pre-release jika masih untuk pengujian.
6. Perbarui versi, nama file, ukuran, serta tautan unduh di README saat rilis berikutnya terbit. Sertakan checksum unduhan bila tersedia.

Push ke `main` untuk perubahan dokumentasi tidak menjalankan build installer; workflow saat ini berjalan melalui tag atau pemicu manual. Jangan memindahkan tag rilis lama untuk perubahan README.

Publikasi draft dapat memicu build lagi saat tag dibuat. Jika commit yang sama sudah berhasil dibangun, batalkan job duplikat agar aset/checksum rilis tidak berubah tanpa perlu.

## Menyiapkan GitHub dan kredit

Upload source, konfigurasi build, dokumentasi, dan lisensi komponen. .gitignore mengecualikan instalasi lokal, runtime staging, binary, .toolchain, database, log, dan file key. Jangan upload folder data, cookie, private key, atau API key. File yang pernah dilacak masih perlu diperiksa; .gitignore tidak menghapus rahasia dari riwayat.

Branding dan metadata pengembang menggunakan **KrisnaZxx**. Hak cipta/lisensi font, ikon, Whisper, Flow2API, dan komponen lain tetap dipertahankan di resources/licenses, flow2api-src/LICENSE, serta notices runtime. Nama lama khusus migrasi folder/keyring diperlukan agar data lama tetap terbaca; itu bukan kredit antarmuka.
