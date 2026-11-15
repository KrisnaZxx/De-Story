# De Story

**by KrisnaZxx** — aplikasi desktop untuk membuat video cerita dari ide, naskah, gambar, narasi, caption, hingga ekspor MP4. Antarmuka berbahasa Indonesia. Proyek/media tersimpan di komputer; layanan AI memakai akun milik pengguna.

## Fitur

- Empat tahap: Ide cerita → Naskah → Storyboard → Editor.
- Durasi hingga 30 menit, YouTube 16:9 atau Shorts 9:16, gaya visual dan pemeran konsisten.
- Naskah melalui Gemini, OpenRouter, Groq, atau endpoint custom; narasi Gemini/ElevenLabs dan preview suara.
- Gambar/video Google Flow, Higgsfield API/MCP, dan Vidgen; multiakun Flow serta Vidgen.
- Whisper lokal/VPS, koreksi teks dan waktu caption, posisi caption yang bisa ditarik di preview.
- Undo/redo, timeline yang bisa diperbesar dan digeser, gerak kamera, transisi, overlay, musik, dan preview layar penuh.
- Thumbnail YouTube otomatis khusus 16:9, memakai model gambar proyek; unduhan PNG terpisah dari video.
- Ekspor FFmpeg lokal atau SSH/VPS, beberapa profil VPS, unduh/hapus hasil, dan informasi storage.
- Windows desktop serta konfigurasi paket GUI Linux .deb/.AppImage.

## Instalasi

### Windows

1. Unduh installer Windows dari Releases repository ini bila sudah tersedia, jalankan, lalu buka De Story dari shortcut.
2. Pastikan WebView2 Runtime tersedia. Windows yang belum memilikinya dapat meminta unduhan WebView2.
3. Buka Pengaturan → Layanan AI dan kunci, lalu siapkan layanan yang digunakan.

### Linux Ubuntu / Debian dengan desktop

Ini aplikasi GUI. Installer Windows tidak dapat dijadikan paket Linux hanya dengan mengganti ekstensi.

1. Gunakan x86_64, Ubuntu 22.04+ atau Debian 12+ dengan desktop dan WebKitGTK 4.1. Workflow rilis menggunakan Ubuntu 22.04 sebagai dasar kompatibilitas.
2. Unduh .deb, buka melalui Software Install/pengelola paket grafis, instal beserta dependensi, lalu buka De Story dari menu aplikasi.
3. Secret Service/keyring desktop harus aktif dan terbuka untuk menyimpan API key.
4. Alternatif AppImage: Properties → Permissions → izinkan executable, lalu klik dua kali. FFmpeg, OpenSSH, WebKitGTK, dan pustaka Chromium tetap diperlukan pada sistem. Paket .deb lebih disarankan karena memasang dependensi.

Build pada Debian yang lebih baru tidak otomatis berjalan di Ubuntu lama. Untuk distribusi di atas, gunakan hasil workflow Ubuntu 22.04; lihat catatan rilis untuk status pengujian sebenarnya.

## Alur membuat video

1. Klik Proyek baru, isi ide/ringkasan cerita.
2. Pilih bahasa, durasi dari dropdown, rasio, gaya visual, penyedia TTS, dan suara. Dengarkan preview sebelum memilih suara.
3. Pada Model AI, pilih model gambar dan model video. Penyedia gambar/video boleh berbeda.
4. Klik Susun naskah. Cerita panjang diproses bertahap; lama proses mengikuti koneksi, panjang naskah, dan model.
5. Di Naskah, periksa judul adegan, narasi, prompt, pemeran, dan urutan. Mengubah naskah setelah TTS dibuat memerlukan rekam ulang agar suara sesuai.
6. Di Storyboard, buat lembar karakter sebagai referensi konsistensi, kemudian generate gambar yang belum tersedia.
7. Buat suara narasi. Satu rangkaian rekaman dipotong per adegan; durasi akhir mengikuti narasi dan bisa berbeda dari durasi awal.
8. Gunakan gerak kamera tanpa video AI, atau Jadikan video AI untuk klip tertentu/semua klip setelah gambar dan suara siap.
9. Buka Editor untuk memeriksa caption, durasi, transisi, musik, dan overlay, kemudian ekspor.

Simpan otomatis berjalan setelah perubahan; Ctrl+S menyimpan langsung. Mode sleep menghentikan aplikasi: biarkan komputer aktif selama generate, Whisper, unggah/render/unduh VPS.

## Pengaturan layanan AI

Semua penyedia ada di Pengaturan → Layanan AI dan kunci, melalui tab Naskah, Suara TTS, Google Flow, Higgsfield, Vidgen, atau Caption Whisper. Menambahkan akun/key tidak otomatis mengganti model proyek lama; pilih Model AI di proyek.

### Naskah dan TTS

Isi key penyedia, pilih model, dan periksa koneksi. Gemini mendukung pool beberapa key yang sah dan menangani status kuota/server tiap key. Biaya serta kuota mengikuti masing-masing penyedia; angka kredit antarpenyedia tidak setara.

Jika Susun naskah timeout, periksa model, koneksi, kuota, serta rincian error. Coba durasi pendek untuk membedakan gangguan koneksi/model dari beban naskah panjang.

### Google Flow / Flow2API

1. Pilih Flow2API internal dan tunggu server siap. Buka panel admin; kredensial panel lokal dapat ditampilkan di pengaturan.
2. Login Google Flow melalui browser. Tambahkan sesi/cookie sesuai format yang diminta panel lokal Flow2API → Token Management. Jangan membagikan cookie/sandi.
3. Untuk extension worker, buka folder ekstensi melalui aplikasi dan pasang di Chrome. Isi alamat/kredensial worker dari pengaturan, simpan, pastikan Terhubung. Setelah pembaruan ekstensi, klik Reload pada chrome://extensions.
4. Alternatif captcha browser lokal membuka Chrome for Testing. Browser diperlukan saat proses berjalan, dan dibersihkan sekitar satu menit setelah tidak digunakan. Browser bisa muncul lagi saat generate berikutnya.
5. Tambahkan akun sah lainnya di Token Management untuk multiakun. Pilih Pengaturan multiakun Flow → jumlah akun atau Semua akun tersambung. Dropdown langsung tersimpan.
6. Di proyek pilih model gambar/video Google Flow; penambahan akun saja belum mengganti model proyek.

Banyak akun meningkatkan paralelisme hanya jika akun dan server tersedia. Sesi/cookie mengikuti ketentuan Google dan dapat kedaluwarsa/dicabut; aplikasi tidak bisa menjamin masa aktif lebih lama. Login ulang/perbarui sesi ketika autentikasi gagal. Status ekstensi Terhubung belum menjamin akun maupun captcha siap.

Layanan captcha tim opsional memerlukan variabel lingkungan `FLOW2API_TEAM_CAPTCHA_CLIENT_KEY` milik operator, lalu restart aplikasi/server. Source publik tidak menyertakan kunci layanan tersebut. Mode captcha browser lokal dan ekstensi tetap tersedia.

### Higgsfield API / MCP

API: isi key, periksa, lalu pilih model Higgsfield. MCP: klik Hubungkan Higgsfield MCP, selesaikan login browser, kemudian pilih model berlabel MCP. Trial, Unlimited, dan akses model mengikuti hak akun penyedia; login berhasil tidak berarti trial aktif.

### Vidgen gambar dan video

1. Buka tab Vidgen, tambahkan nama akun dan API key vg_…, lalu Periksa akun. Tambahkan akun berbeda lainnya jika ada.
2. Pilih maksimal akun paralel atau Semua akun aktif yang tersambung. Dropdown/sakelar langsung tersimpan. Dua key dari akun sama tetap dihitung satu akun.
3. Pilih model gambar Vidgen pada Model AI untuk gambar, lembar karakter, dan thumbnail. Pilih Omni 1.2 Flash Unlimited · Vidgen untuk image to video.

**Gambar Vidgen memakai kredit berbayar.** Aplikasi menampilkan perkiraan harga dan memeriksa katalog langsung serta saldo sebelum mengirim permintaan. Jika harga naik dari katalog aplikasi, proses dihentikan agar tagihan tidak melebihi perkiraan. Perbarui katalog aplikasi bila harga/model berubah.

Video Omni 1.2 memakai paket Unlimited aktif dan tetap mengikuti antrean, batas harian, dan ketentuan server. De Story menjalankan satu pekerjaan media per akun; akun berbeda dapat berjalan bersamaan. Mengantre banyak job bukan berarti semua dirender bersamaan. Hasil segera diunduh ke proyek. Pembatalan di aplikasi tidak menjamin job yang sudah diterima berhenti di server; periksa riwayat sebelum mengulang pekerjaan yang statusnya belum jelas.

Referensi: [katalog Vidgen](https://vidgen.id/api/v1/models), [dokumentasi API](https://docs.vidgen.id/llms.txt).

## Caption, klip, dan editor

### Whisper lokal / VPS

Buka Layanan AI → Caption Whisper dan pilih Lokal atau VPS. Lokal memerlukan unduhan model Whisper Small dan memakai CPU komputer. Untuk VPS, simpan profil SSH, pilih VPS untuk Whisper di Setting SSH, lalu pasang/periksa Whisper VPS melalui tab Caption Whisper. Pengenalan membutuhkan waktu dan tetap mungkin salah.

### Caption terlambat / tidak sesuai

1. Editor → Caption, pilih klip.
2. Sinkronkan dengan Whisper untuk membuat ulang waktu dari audio yang ada; ini tidak meminta TTS baru.
3. Geser seluruh caption menggunakan offset detik: -0.3 memajukan 0,3 detik; 0.3 menunda.
4. Edit teks caption, klik di luar kolom untuk menerapkan. Jumlah kata sama mempertahankan timing; jumlah berbeda membagikan ulang waktu dalam rentang lama.
5. Edit waktu per kata untuk mengatur mulai/akhir relatif terhadap awal klip.
6. Tarik caption di timeline ke kiri/kanan untuk menggeser waktu, dalam batas klip dan kata tetangga.
7. Saat tab Caption aktif, tarik teks pada preview untuk memindahkan posisi caption. Atas/Tengah/Bawah mengembalikan posisi preset.

Edit berlaku pada preview, video dengan caption, dan SRT. Edit terikat sumber audio; merekam ulang suara membuat edit audio lama tidak berlaku pada audio baru. Pulihkan caption klip mengembalikan waktu dari audio aktif; offset global tetap berlaku sampai diatur ke 0.

### Klip terlalu pendek

Preview/ekspor menjaga durasi setidaknya sepanjang audio. Ubah Editor → Klip → Durasi klip untuk memperpanjang visual. Video AI yang lebih pendek menahan frame terakhir sampai narasi selesai.

Untuk rekaman Gemini utuh yang salah dipotong, gunakan Editor → Caption → Perbaiki pembagian klip dari audio lama. Aplikasi menyatukan audio lama, menjalankan Whisper, lalu memotong ulang tanpa TTS baru. Semua klip harus lengkap, berasal dari take sama, dan naskah belum berubah. Versi audio lama tetap disimpan. Bila syarat tidak terpenuhi atau TTS melewatkan kalimat, perbaiki naskah lalu rekam ulang narasi.

Sesudah durasi berubah, periksa kembali posisi overlay/musik yang menggunakan waktu absolut.

### Timeline dan preview

- Klik klip untuk memilih dan memindahkan playhead; klik caption membuka edit caption.
- Zoom memperbesar detail. Geser timeline lewat slider bawah, panah kiri/kanan, scrollbar, atau Shift+roda mouse. Semua track bergerak bersama.
- Gulir area track ke bawah bila track musik/overlay belum terlihat. Saat putar, timeline mengikuti playhead.
- Transisi ditampilkan jika ruang cukup atau klip dipilih agar tombol tidak bertumpuk.
- Perbesar preview menyembunyikan panel/timeline sementara; tekan lagi untuk kembali.
- Preview layar penuh membuka fullscreen; Esc keluar. Spasi memutar/jeda di luar input.
- Ctrl+Z / Urungkan membatalkan edit; Ctrl+Shift+Z atau Ctrl+Y mengulang. Maksimal 50 langkah selama proyek terbuka. Input teks memakai undo bawaan input.
- Undo tidak membatalkan job AI, kredit, atau file ekspor. Media AI yang selesai dibuat tetap dipertahankan.

## Ekspor lokal / VPS

### Lokal

Ekspor video → Lokal, lalu pilih resolusi, FPS, caption, SRT bila perlu, dan folder hasil. Folder bawaan hasil-story. FFmpeg menggabungkan media, transisi, audio, dan caption. Setelah selesai gunakan Ekspor ulang untuk versi baru; file sebelumnya tidak ditimpa otomatis.

### Beberapa VPS

1. Pengaturan → Setting SSH → Tambah VPS.
2. Isi alias unik, host, port, username, dan pilih private key lokal jika tidak memakai SSH agent. Simpan, ulangi untuk VPS lain.
3. Tentukan VPS bawaan ekspor dan VPS untuk Whisper. Pilihan Whisper kosong mengikuti VPS ekspor.
4. Periksa SSH, kemudian Pasang kebutuhan VPS untuk FFmpeg/worker. Installer otomatis mendukung Debian/Ubuntu dengan sudo tanpa prompt.

OpenSSH harus tersedia dan fingerprint host sudah dipercaya di komputer ini. Pada koneksi pertama, cocokkan fingerprint dengan penyedia VPS sebelum mempercayainya lewat SSH. Aplikasi memakai pemeriksaan host ketat. Private key tetap di komputer, tidak diunggah ke VPS.

### Render / hasil VPS

1. Ekspor video → VPS melalui SSH, pilih alias tersimpan atau alias konfigurasi SSH sistem, lalu periksa koneksi.
2. Aktifkan Hapus video di VPS setelah download jika diinginkan.
3. Aplikasi mengunggah aset, merender dengan FFmpeg di VPS, mengunduh MP4, lalu memverifikasi SHA-256. Penghapusan hanya dilakukan setelah unduhan berhasil diverifikasi.
4. Setting SSH → pilih profil → Cek semua hasil video untuk melihat storage, unduh hasil lain, atau hapus hasil yang tidak diperlukan.

Caption dibuat sebelum render; Whisper dan FFmpeg VPS tidak harus berjalan bersamaan. VPS menambah transfer upload aset/download hasil. Aplikasi dan koneksi tetap diperlukan selama proses. Kecepatan mengikuti CPU, resolusi/FPS, ukuran aset, dan jaringan.

## Thumbnail YouTube

Khusus proyek 16:9: Editor → Thumbnail otomatis, sebelum atau sesudah ekspor. AI memilih adegan dan teks singkat dari cerita, lalu model gambar proyek membuat thumbnail beserta teks. Unduh PNG 1280 × 720 terpisah; thumbnail tidak digabung ke MP4. Generasi ulang memakai kuota/kredit gambar. Periksa ejaan hasil AI sebelum publikasi. Proyek 9:16 tidak menampilkan fitur ini.

## Data dan backup

Windows: %APPDATA%\De Story. Linux: $XDG_DATA_HOME/De Story, biasanya ~/.local/share/De Story. Folder memuat studio.db, media, model Whisper, dan pengaturan/kredensial lokal.

Tutup aplikasi sebelum backup seluruh folder. Menyalin database saja tidak menyertakan media. Key terenkripsi mengikuti pengguna/komputer; perpindahan OS bisa memerlukan login/key ulang. Pembaruan tidak perlu menghapus data. Backup sebelum mengganti versi.

## Build dari source

Prasyarat: Node.js 22+, npm, Rust stable, Git. Windows memerlukan C++ Build Tools dan WebView2; Linux mengikuti [prasyarat Tauri](https://v2.tauri.app/start/prerequisites/#linux).

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

Whisper UNIX dikompilasi dari commit yang dipatok di scripts/build-whisper-unix.sh, tanpa optimasi CPU khusus mesin build. End user cukup memakai installer GUI. Untuk pengembangan: build Whisper pada UNIX, stage Flow2API, lalu npm run tauri:dev.

Hasil paket ada di src-tauri/target/release/bundle kecuali CARGO_TARGET_DIR diubah. Workflow .github/workflows/release.yml membangun Windows/macOS/Linux dan membuat draft release untuk diperiksa sebelum terbit.

```bash
node scripts/editor-checks.mjs
node scripts/vidgen-ui-checks.mjs
cargo test --lib --locked --manifest-path src-tauri/Cargo.toml
```

## Menyiapkan GitHub dan kredit

Upload source, konfigurasi build, dokumentasi, dan lisensi komponen. .gitignore mengecualikan instalasi lokal, runtime staging, binary, .toolchain, database, log, dan file key. Jangan upload folder data, cookie, private key, atau API key. File yang pernah dilacak masih perlu diperiksa; .gitignore tidak menghapus rahasia dari riwayat.

Branding dan metadata pengembang menggunakan **KrisnaZxx**. Hak cipta/lisensi font, ikon, Whisper, Flow2API, dan komponen lain tetap dipertahankan di resources/licenses, flow2api-src/LICENSE, serta notices runtime. Nama lama khusus migrasi folder/keyring diperlukan agar data lama tetap terbaca; itu bukan kredit antarmuka.
