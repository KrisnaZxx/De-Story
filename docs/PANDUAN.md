# Panduan Bara Studio

[Kembali ke README](../README.md) · [Unduh aplikasi](https://github.com/KrisnaZxx/De-Story/releases)

Langsung ke: [Flow](#google-flow) · [Caption dan audio](#caption-klip-dan-editor) · [Ekspor dan VPS](#ekspor-lokal-dan-vps) · [Masalah umum](#masalah-umum).

## Alur membuat video

1. Klik Proyek baru, isi ide/ringkasan cerita.
2. Pilih bahasa, durasi dari dropdown, rasio, gaya visual, penyedia TTS, dan suara. Dengarkan preview sebelum memilih suara.
3. Pada Model AI, pilih model gambar dan model video. Penyedia gambar/video boleh berbeda.
4. Klik Susun naskah. Cerita panjang diproses bertahap; lama proses mengikuti koneksi, panjang naskah, dan model.
5. Di Naskah, periksa judul adegan, narasi, prompt, pemeran, dan urutan. Mengubah naskah setelah TTS dibuat memerlukan rekam ulang agar suara sesuai.
6. Di Storyboard, buat lembar karakter sebagai referensi konsistensi, kemudian generate gambar yang belum tersedia.
7. Buat suara narasi. Gemini/ElevenLabs menggabungkan rekaman lalu memotongnya per adegan; Speko TTS membuat suara tiap klip dengan waktu kata bawaan; durasi akhir mengikuti narasi dan bisa berbeda dari durasi awal.
8. Gunakan gerak kamera tanpa video AI, atau Jadikan video AI untuk klip tertentu/semua klip setelah gambar dan suara siap.
9. Buka Editor untuk memeriksa caption, durasi, transisi, musik, dan overlay, kemudian ekspor.

Simpan otomatis berjalan setelah perubahan; Ctrl+S menyimpan langsung. Mode sleep menghentikan aplikasi: biarkan komputer aktif selama generate, Whisper, unggah/render/unduh VPS.

## Pengaturan layanan AI

Semua penyedia ada di Pengaturan → Layanan AI dan kunci, melalui tab Naskah, Suara TTS, Google Flow, Higgsfield, Vidgen, Speko, atau Caption. Menambahkan akun/key tidak otomatis mengganti model proyek lama; pilih Model AI di proyek.

### Naskah dan TTS

Isi key penyedia, pilih model, dan periksa koneksi. Gemini mendukung pool beberapa key yang sah dan menangani status kuota/server tiap key. Biaya serta kuota mengikuti masing-masing penyedia; angka kredit antarpenyedia tidak setara.

Jika Susun naskah timeout, periksa model, koneksi, kuota, serta rincian error. Coba durasi pendek untuk membedakan gangguan koneksi/model dari beban naskah panjang.

### Speko TTS dan STT

**Isi kunci dan model:** Pengaturan → Layanan AI dan kunci → Speko. Masukkan **Router API key** `sk_speko_…`, lalu simpan/periksa. Pemeriksaan membaca katalog; tidak membuat suara atau transkripsi berbayar. Klik muat ulang model setelah akses akun berubah.

Daftar mengikuti katalog akun, region, dan ketersediaan layanan. TTS hanya menampilkan model streaming dengan **word timings**; STT hanya model batch dengan **word timestamps**. Jika daftar kosong, akses akun belum menyediakan kemampuan tersebut. Aplikasi tidak memilih model pengganti secara diam-diam. Pilih bahasa proyek **Indonesia** untuk audio Indonesia; dukungan dan kualitas tetap mengikuti model yang dipilih.

| Alur | Pengaturan | Hasil |
| --- | --- | --- |
| Gemini TTS + Speko STT | Model STT di tab Speko; tab Caption → mesin **Speko STT** → aktifkan **Sinkronkan otomatis**; proyek tetap Gemini TTS | Suara dari Gemini, waktu caption/pembagian adegan dari Speko |
| Speko TTS | Model TTS di tab Speko; proyek → Ide cerita → suara **Speko TTS** | Suara dan waktu kata bawaan, tanpa STT tambahan |
| Audio lama + Speko STT | Pilih mesin Speko STT; Editor → Caption → **Sinkronkan dengan Speko STT** | Audio lama tetap; waktu caption diperbarui |

Di langkah **Ide cerita**, pilih Speko lalu buka **Suara narator**. Daftar mengikuti model: Soniox, Cartesia Sonic, atau ElevenLabs. Pilih **Otomatis sesuai bahasa** atau cari nama suara; suara Cartesia Indonesia diprioritaskan. Tombol ▶ memutar contoh yang sudah tersedia dari penyedia, tanpa membuat TTS berbayar. Bahasa contoh tercantum bila diketahui; bahasa narasi tetap mengikuti proyek. Soniox belum menyediakan contoh siap putar melalui katalog ini. Daftar bawaan dapat tertinggal dari katalog penyedia.

Audio STT dikonversi ke WAV mono 16 kHz, dibagi maksimal 45 detik (atau lebih kecil mengikuti batas model), lalu waktunya digabung. Tumpang tindih satu detik membantu kata di batas potongan dan menambah sedikit durasi yang ditagih. Pemrosesan rekaman sampai 120 menit memakai unggahan bertahap tanpa webhook publik. Sinkronisasi dapat dibatalkan; bagian yang sudah diterima penyedia dapat tetap ditagih.

**Biaya:** Speko TTS dihitung per karakter; STT per durasi audio. Gemini + Speko memakai tagihan kedua layanan dan belum tentu paling murah. Sinkronisasi ulang menagih STT lagi. Speko TTS/ElevenLabs memakai waktu bawaan sehingga tidak otomatis ditranskripsi ulang. Bila STT gagal setelah suara Gemini selesai, audio tetap disimpan dengan penanda caption perkiraan; sinkronkan lagi tanpa membuat ulang TTS. Permintaan gagal tidak otomatis diulang atau dialihkan ke model lain.

Kunci disimpan terenkripsi di backend perangkat, tidak disertakan dalam installer. Audio/naskah dikirim ke Speko saat memakai layanan tersebut. Jangan unggah key ke GitHub atau kirim melalui chat. Integrasi diuji memakai server tiruan; uji akun/model berbayar perlu dilakukan dengan key Anda melalui aplikasi.

Dokumentasi resmi: [model](https://docs.speko.ai/relay/models), [TTS streaming](https://docs.speko.ai/relay/tts/streaming), [STT batch](https://docs.speko.ai/relay/stt/batch).

### Tema aplikasi

Buka **Pengaturan → Umum → Tema aplikasi**, pilih **Gelap · Graphite** atau **Terang · Pearl**. Perubahan langsung terlihat dan disimpan. Tema memengaruhi antarmuka; warna gambar, caption video, thumbnail, dan hasil ekspor mengikuti pengaturan proyek.

### Google Flow

1. Pilih Flow2API internal dan tunggu server siap. Buka panel admin; kredensial panel lokal dapat ditampilkan di pengaturan.
2. Pada **Sesi login Flow**, klik **Tambah profil akun → Buka profil / login**. Login manual pada Chrome/Chromium reguler yang terpasang, selesaikan verifikasi, dan pastikan halaman Google Flow sudah terbuka. **Tutup jendela profil login melalui tombol X**, lalu kembali ke Bara Studio dan klik **Selesai login dan simpan**. Aplikasi baru membaca sesi setelah jendela tersebut ditutup. Tidak perlu memasukkan password Google ke aplikasi.
3. Untuk extension worker, buka folder ekstensi melalui aplikasi dan pasang di Chrome. Isi alamat/kredensial worker dari pengaturan, simpan, pastikan Terhubung. Setelah pembaruan ekstensi, klik Reload pada chrome://extensions.
4. Alternatif captcha browser lokal membuka Chrome for Testing. Browser diperlukan saat proses berjalan, dan dibersihkan sekitar satu menit setelah tidak digunakan. Browser bisa muncul lagi saat generate berikutnya.
5. Ulangi Tambah profil akun untuk akun lain. Pilih Pengaturan multiakun Flow → jumlah akun atau Semua akun tersambung. Dropdown langsung tersimpan. Akun yang sebelumnya diimpor lewat Token Management tetap didukung.
6. Di proyek pilih model gambar/video Google Flow; penambahan akun saja belum mengganti model proyek.

Untuk akun lama, klik **Tautkan semua akun tersimpan** sekali. Aplikasi menyiapkan profil browser terpisah dan mencoba memakai cookie yang sudah tersedia. Akun dengan cookie kedaluwarsa mungkin masih memerlukan login awal pada profilnya.

Aktifkan **Perbarui sesi otomatis** dan pilih interval, misalnya 30 menit. Aplikasi mencoba pembaruan melalui jaringan dahulu; bila perlu, memakai profil tersimpan satu per satu agar tidak membuka banyak browser. Browser ditutup setelah penyegaran; data login tetap tersimpan. Gunakan **Perbarui semua sesi** dan filter **Tampilkan hanya akun yang perlu tindakan** untuk banyak akun. Saat Google meminta verifikasi, cukup buka profil akun bermasalah, login manual, tutup jendelanya, dan simpan kembali sesinya. Jendela login biasa ditutup sendiri oleh pengguna.

Jika muncul **This browser or app may not be secure**, pastikan memakai alur login manual versi 1.2.1 atau lebih baru, Chrome reguler terbaru, dan JavaScript aktif. [Google dapat menolak browser yang dikendalikan otomatis saat login](https://support.google.com/accounts/answer/7675428?hl=id). Login dilakukan sebelum aplikasi menghubungkan profil untuk membaca sesi. Jika tetap ditolak, ikuti pemulihan/verifikasi Google atau gunakan impor cookie dari sesi browser biasa melalui panel admin; jangan menonaktifkan perlindungan akun.

**Kredit dan pemakaian:** buka **Pengaturan → Layanan AI dan kunci → Google Flow → Pengaturan multiakun Flow → Perbarui akun & saldo**. Tabel menampilkan saldo tiap akun dan perubahan saldo video terakhir. Pemakaian dicatat dari saldo sebelum/sesudah generasi, sehingga aktivitas lain pada akun yang sama dapat memengaruhinya; angka yang belum terkonfirmasi tidak ditampilkan sebagai nol. Estimasi video di Storyboard memakai tarif model dan batas atas untuk campuran paket akun, bukan selalu 0 kredit. [Tarif resmi Flow](https://support.google.com/flow/answer/16526234?hl=en) dapat berubah.

**Akun kehabisan kredit/kuota:** akun yang ditolak secara jelas dilewati untuk jenis media tersebut selama hingga satu jam. Pekerjaan beralih ke akun lain setelah slot sebelumnya dilepas. Jika hanya empat dari lima akun siap, batas efektif menjadi empat. Saat saldo bertambah setelah diperbarui, akun dapat dipakai lagi. Galat koneksi yang membuat status pengajuan tidak pasti tidak memicu generasi ulang lintas akun.

Banyak akun meningkatkan paralelisme hanya jika akun dan server tersedia. Sesi/cookie mengikuti ketentuan Google dan dapat kedaluwarsa/dicabut; aplikasi tidak bisa menjamin masa aktif tertentu. Pembaruan otomatis memerlukan Bara Studio aktif dan komputer tidak sleep. OAuth Google biasa tidak menggantikan sesi login situs Flow. Status ekstensi Terhubung belum menjamin akun maupun captcha siap.

Layanan captcha tim opsional memerlukan variabel lingkungan `FLOW2API_TEAM_CAPTCHA_CLIENT_KEY` milik operator, lalu restart aplikasi/server. Source publik tidak menyertakan kunci layanan tersebut. Mode captcha browser lokal dan ekstensi tetap tersedia.

### Higgsfield API / MCP

API: isi key, periksa, lalu pilih model Higgsfield. MCP: klik Hubungkan Higgsfield MCP, selesaikan login browser, kemudian pilih model berlabel MCP. Trial, Unlimited, dan akses model mengikuti hak akun penyedia; login berhasil tidak berarti trial aktif.

### Vidgen gambar dan video

1. Buka tab Vidgen, tambahkan nama akun dan API key vg_…, lalu Periksa akun. Tambahkan akun berbeda lainnya jika ada.
2. Pilih maksimal akun paralel atau Semua akun aktif yang tersambung. Dropdown/sakelar langsung tersimpan. Dua key dari akun sama tetap dihitung satu akun.
3. Pilih model gambar Vidgen pada Model AI untuk gambar, lembar karakter, dan thumbnail. Pilih Omni 1.2 Flash Unlimited · Vidgen untuk image to video.

**Gambar Vidgen memakai kredit berbayar.** Aplikasi menampilkan perkiraan harga dan memeriksa katalog langsung serta saldo sebelum mengirim permintaan. Jika harga naik dari katalog aplikasi, proses dihentikan agar tagihan tidak melebihi perkiraan. Perbarui katalog aplikasi bila harga/model berubah.

Video Omni 1.2 memakai paket Unlimited aktif dan tetap mengikuti antrean, batas harian, dan ketentuan server. Bara Studio menjalankan satu pekerjaan media per akun; akun berbeda dapat berjalan bersamaan. Mengantre banyak job bukan berarti semua dirender bersamaan. Hasil segera diunduh ke proyek. Pembatalan di aplikasi tidak menjamin job yang sudah diterima berhenti di server; periksa riwayat sebelum mengulang pekerjaan yang statusnya belum jelas.

Referensi: [katalog Vidgen](https://vidgen.id/api/v1/models), [dokumentasi API](https://docs.vidgen.id/llms.txt).

## Caption, klip, dan editor

### Whisper lokal / VPS

Buka **Layanan AI dan kunci → Caption** dan pilih **Lokal** atau **VPS**.

- **Lokal:** program Whisper ikut installer. Klik **Unduh model** untuk mengambil model Small ±488 MB satu kali. Setelah model tersedia, proses Whisper berjalan di komputer tanpa layanan AI berbayar.
- **VPS:** simpan koneksi di **Pengaturan SSH**, pilih VPS untuk Whisper, lalu klik **Pasang Whisper di VPS** dan **Periksa Whisper VPS** pada tab Caption. Model diunduh langsung ke VPS; model lokal tidak diperlukan.
- **Sinkronkan otomatis:** aktifkan untuk memproses timing setelah narasi Gemini dibuat. Jika dimatikan atau Whisper belum siap, narasi tetap bisa dibuat dengan timing perkiraan, lalu disinkronkan kemudian.

Gemini TTS menghasilkan audio; Whisper membaca audio itu untuk mencari waktu kata dan batas adegan. Jalur ElevenLabs dan Speko TTS memakai timing kata dari penyedia. Whisper tidak membuat suara baru dan hasilnya tetap mungkin keliru. Pilihan lokasi Whisper terpisah dari lokasi ekspor video.

### Caption terlambat / tidak sesuai

1. Editor → Caption, pilih klip.
2. Sinkronkan dengan Whisper atau Speko STT (sesuai pengaturan) untuk membuat ulang waktu dari audio yang ada; ini tidak meminta TTS baru.
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

## Ekspor lokal dan VPS

### Lokal

Ekspor video → Lokal, lalu pilih resolusi, FPS, caption, SRT bila perlu, dan folder hasil. Folder bawaan hasil-story. Klik **Mulai ekspor**: setelah job diterima, dialog pengaturan menutup otomatis. FFmpeg menggabungkan media, transisi, audio, dan caption di latar aplikasi.

Buka **Riwayat ekspor** di bagian atas proyek untuk melihat antrean, persentase, status selesai/gagal, resolusi/FPS, tujuan lokal/VPS, serta folder hasil. Riwayat tetap tersimpan ketika proyek dibuka kembali. Satu proyek menjalankan satu ekspor pada satu waktu. Setelah selesai atau dibatalkan, gunakan **Ekspor ulang** untuk membuka pengaturan baru; file sebelumnya tidak ditimpa otomatis. Menutup dialog tidak membatalkan ekspor, tetapi menutup aplikasi/sleep dapat menghentikan proses.

### Beberapa VPS

1. Pengaturan → Pengaturan SSH → Tambah VPS.
2. Isi alias unik, host, port, dan username. Pilih **SSH key / agent** (pilih private key lokal bila diperlukan) atau **Password akun VPS**, lalu isi password. Simpan, ulangi untuk VPS lain.
3. Tentukan VPS bawaan ekspor dan VPS untuk Whisper. Pilihan Whisper kosong mengikuti VPS ekspor.
4. Periksa SSH, kemudian Pasang kebutuhan VPS untuk FFmpeg/worker. Installer otomatis mendukung Debian/Ubuntu sebagai root atau dengan sudo tanpa prompt. Password login SSH tidak otomatis menjawab prompt sudo/OTP.

OpenSSH harus tersedia dan fingerprint host sudah dipercaya di komputer ini. Pada koneksi pertama, cocokkan fingerprint dengan penyedia VPS sebelum mempercayainya lewat SSH. Aplikasi memakai pemeriksaan host ketat. Private key tetap di komputer, tidak diunggah ke VPS.

Password disimpan terenkripsi memakai DPAPI Windows. Password tidak ditaruh pada konfigurasi SSH, argumen perintah, atau screenshot. Kolom kosong saat mengedit mempertahankan password tersimpan untuk tujuan yang sama; perubahan host, port, atau username memerlukan pengisian ulang. Server yang mewajibkan verifikasi tambahan tetap memerlukan alur verifikasi yang sesuai.

### Render / hasil VPS

1. Ekspor video → VPS melalui SSH, pilih alias tersimpan atau alias konfigurasi SSH sistem, lalu periksa koneksi.
2. Aktifkan Hapus video di VPS setelah download jika diinginkan.
3. Aplikasi mengunggah aset, merender dengan FFmpeg di VPS, mengunduh MP4, lalu memverifikasi SHA-256. Penghapusan hanya dilakukan setelah unduhan berhasil diverifikasi.
4. Pengaturan SSH → pilih profil → Cek semua hasil video untuk melihat storage, unduh hasil lain, atau hapus hasil yang tidak diperlukan.

Caption dibuat sebelum render; Whisper dan FFmpeg VPS tidak harus berjalan bersamaan. VPS menambah transfer upload aset/download hasil. Aplikasi dan koneksi diperlukan untuk unggah, pemantauan, serta unduh otomatis; render yang sudah dimulai tetap berjalan saat SSH terputus. Kecepatan mengikuti CPU, resolusi/FPS, ukuran aset, dan jaringan.

## Thumbnail YouTube

Khusus proyek 16:9: **Editor → Thumbnail**, sebelum atau sesudah ekspor. AI memilih adegan dan teks singkat dari cerita, lalu model gambar proyek membuat thumbnail beserta teks. Unduh PNG 1280 × 720 terpisah; thumbnail tidak digabung ke MP4. Generasi ulang memakai kuota/kredit gambar. Periksa ejaan hasil AI sebelum publikasi. Proyek 9:16 tidak menampilkan fitur ini.

## Deskripsi dan tag YouTube

Khusus proyek 16:9: Editor → **Deskripsi & tag → Buat deskripsi dan tag**. Model AI naskah membuat deskripsi singkat dan tag dari judul, ringkasan, serta narasi video. Hasil tersimpan di proyek, bisa diedit dan disalin terpisah untuk YouTube; tidak dimasukkan ke MP4 dan tidak mengunggah video otomatis. Generate memakai kuota model naskah. Bila cerita berubah, buat ulang agar tetap sesuai. Fitur ini terpisah dari thumbnail dan ekspor.

## Data dan backup

Folder data Windows ada di `%APPDATA%\De Story`. Folder ini memuat `studio.db`, media, model Whisper, dan pengaturan/kredensial lokal.

Tutup aplikasi sebelum backup seluruh folder. Menyalin database saja tidak menyertakan media. Key terenkripsi mengikuti pengguna/komputer; perpindahan OS bisa memerlukan login/key ulang. Pembaruan tidak perlu menghapus data. Backup sebelum mengganti versi.

## Masalah umum

| Masalah | Yang dilakukan |
| --- | --- |
| Bingung memilih file unduhan | Pilih satu installer untuk OS/arsitektur di [README](../README.md#unduh-aplikasi). `.exe` dan `.msi` adalah alternatif Windows. Source code bukan installer. |
| Google menampilkan “This browser or app may not be secure” | Gunakan v1.2.1 atau lebih baru. Buka profil dari Bara Studio, login manual di Chrome reguler, tutup jendela profil melalui X, lalu simpan sesi. Perbarui Chrome dan pastikan JavaScript aktif bila masih ditolak. |
| Diminta menutup Chrome sebelum menyimpan sesi | Tutup jendela **profil login Flow tersebut**, lalu klik **Selesai login dan simpan** lagi. |
| Ekstensi Terhubung tetapi generate gagal | Periksa akun Flow, status sesi, model proyek, kuota, dan metode captcha. Sambungan worker saja belum menjamin akun siap. |
| Sesi Flow kedaluwarsa | Aktifkan pembaruan otomatis saat aplikasi aktif. Gunakan filter akun bermasalah; login ulang hanya pada akun yang memerlukannya. Tidak ada jaminan durasi sesi tetap. |
| Susun naskah timeout | Periksa key, model, kuota, koneksi, serta rincian error. Coba proyek pendek untuk membedakan gangguan layanan dari beban naskah panjang. |
| Speko TTS gagal dengan pesan urutan/jenis event | Perbarui ke v1.3.1 atau lebih baru; event pencatatan pemakaian `usage.updated` sudah ditangani. Bila masih gagal, simpan pesan galat dan jangan mengulang seluruh narasi berkali-kali. Teks yang sudah diterima Speko dapat tetap ditagih meski hasil belum tersimpan. |
| Caption lebih telat daripada suara | Sinkronkan ulang dengan Whisper. Offset negatif, misalnya `-0.3`, memajukan caption 0,3 detik. Periksa awal, tengah, dan akhir. |
| Klip terlalu pendek / narasi terpotong | Periksa durasi klip dan audio. Untuk rekaman Gemini yang salah dibagi, lihat [perbaikan pembagian audio](#klip-terlalu-pendek). |
| Dialog ekspor menutup setelah Mulai ekspor | Normal. Buka **Riwayat ekspor** untuk melihat progres dan hasil. Aplikasi harus tetap aktif. |
| SSH gagal / Permission denied | Periksa profil, host, port, username, key/password, dan fingerprint host yang dipercaya. Password SSH berbeda dari izin sudo atau OTP. |
| Model Whisper belum tersedia | Unduh model lokal dari tab Caption, atau pasang dan periksa Whisper di VPS yang dipilih. |
| Whisper atau ekspor lambat | Gunakan pilihan VPS bila tersedia. CPU, resolusi/FPS, ukuran aset, dan jaringan memengaruhi waktu. Unggah/unduh tetap memakai bandwidth. |

Jika belum teratasi, [buat issue](https://github.com/KrisnaZxx/De-Story/issues/new) berisi versi, OS/arsitektur, langkah kejadian, dan pesan error. Sensor informasi akun pada log/screenshot.

### Saat koneksi ekspor VPS putus

Render v1.4.0 berjalan sebagai proses terpisah di VPS. Aplikasi mencoba menyambung ulang dan membaca progres selama hingga 10 menit tanpa menjalankan render kedua. Jika koneksi belum pulih atau aplikasi ditutup, cek **Pengaturan → Pengaturan SSH → Hasil video di VPS** untuk melihat hasil dan mengunduhnya. Pilih hapus setelah unduhan untuk menghemat storage. VPS yang mati/restart tetap dapat menghentikan render.

Ekspor lama yang gagal karena Broken pipe perlu dibuat ulang; file sementara dari proses gagal sebelumnya mungkin sudah dibersihkan. Jangan mengulang ekspor dengan ID baru ketika render sebelumnya masih berjalan.

### Pengaman bahasa Speko (v1.4.1)

Di pilihan model Speko, pilih **Bahasa untuk daftar model Speko**. Daftar hanya menampilkan model yang dukungan bahasanya sudah terkonfirmasi. Saat dibuka dari proyek, filter awal mengikuti bahasa proyek; dari beranda mengikuti bahasa bawaan. Mengubah filter tidak mengubah bahasa proyek, model, atau suara tersimpan secara otomatis.

Contoh: proyek Indonesia tidak dapat memakai ElevenLabs Flash v2 (khusus Inggris), sedangkan Flash v2.5, Multilingual v2, Soniox tts-rt-v2, serta Cartesia Sonic 3/3.5/3.6 mendukung Indonesia. Mengubah bahasa proyek ke Inggris memuat ulang daftar suara; contoh suara yang sesuai diprioritaskan untuk penyedia multibahasa.

Pemeriksaan terakhir dilakukan sebelum permintaan TTS/STT berbayar. Jika Gemini TTS memakai Speko STT otomatis, kecocokan STT diperiksa sebelum membuat suara Gemini. Munsit tidak dapat dipakai untuk caption Indonesia. Model yang belum mempunyai data bahasa terverifikasi, termasuk entri katalog baru, diblokir sampai datanya diperbarui; ini tidak berarti penyedianya pasti tidak mendukung bahasa tersebut. Tidak ada pergantian model secara diam-diam.

Data dukungan diperiksa 16 November 2026; daftar bahasa dan sumbernya ada di [katalog bahasa Speko](../src/shared/speko-languages.json). Pelafalan dan aksen tetap mengikuti model serta suara penyedia.

### Ekspor 2K (1440p)

Pada **Editor → Ekspor video → Resolusi**, pilih **2K / 1440p**. Hasil 16:9 berukuran **2560 × 1440**, sedangkan 9:16 berukuran **1440 × 2560**. Tersedia untuk render laptop maupun VPS melalui SSH, pada 30 atau 60 FPS. Resolusi juga tercatat di Riwayat ekspor. Ukuran file pada dialog hanya perkiraan; ekspor 2K tidak menambah detail yang tidak ada pada gambar/video sumber.

### Membagi render ke beberapa VPS

1. Tambahkan semua VPS di **Pengaturan → Pengaturan SSH**, pasang kebutuhan render, dan periksa koneksi masing-masing.
2. Buka **Ekspor video → VPS melalui SSH**, tentukan **VPS utama untuk penggabungan**.
3. Aktifkan **Bagi render klip ke beberapa VPS**, pilih VPS tambahan, lalu **Periksa VPS terpilih**.
4. Pilih resolusi/FPS dan mulai. Pantau lewat **Riwayat ekspor**; daftar VPS ikut tersimpan pada riwayat.

Klip dibagi menjadi bagian cerita berurutan menurut durasi, lalu dirender bersamaan. VPS utama juga mendapat bagian klip. Tiap VPS menggabungkan bagiannya secara bertahap, maksimal empat input video per proses penggabungan. Overlap transisi dan urutan cerita tetap mengikuti timeline. Jumlah VPS aktif maksimal sebanyak klip; proyek satu klip memakai VPS utama saja.

Mulai v1.5.1, bagian dari VPS B/C dipindahkan **langsung ke VPS utama** melalui port HTTPS privat InstaBox. VPS utama memeriksa ukuran dan SHA-256 sebelum bagian sumber dihapus. Laptop mengirim aset awal serta petunjuk kerja dan menerima video akhir; payload segmen tidak melewati laptop. Port transfer memakai token sementara dan ditutup setelah transfer. Kunci SSH tetap di laptop. [Jalur port privat InstaBox](https://instabox.sh/docs/access/ports).

Transfer langsung multi-VPS saat ini memerlukan `instabox-port` pada VPS sumber. Jika belum tersedia, ekspor berhenti sebelum render dengan pesan yang jelas; tidak diam-diam memakai laptop sebagai perantara. VPS selain InstaBox tetap dapat memakai ekspor satu VPS. VPS utama menerima beberapa bagian yang sudah dirangkai, lalu menerapkan transisi antarbagiannya, caption, narasi, overlay, serta musik dan encoding akhir. Hasilnya tetap memerlukan CPU VPS utama.

Biarkan aplikasi dan koneksi aktif untuk koordinasi dan unduhan akhir. Jika aplikasi ditutup saat pembagian, pekerjaan belum dapat dilanjutkan otomatis dari checkpoint; ekspor ulang setelah memeriksa hasil VPS. Pembatalan menghentikan bagian yang masih berjalan selama VPS dapat dihubungi. Bagian sementara yang tertinggal dapat diperiksa/hapus setelah pekerjaan berhenti.

Dua VPS tidak menjamin dua kali lebih cepat: penggabungan akhir tetap memakai satu VPS, dan transfer tergantung jaringan. Pembagian berdasarkan durasi belum memperhitungkan perbedaan kekuatan CPU; paling sesuai untuk VPS dengan kemampuan yang serupa. Pilihan render laptop dan satu VPS tetap tersedia.

### RAM habis saat penggabungan akhir

Pesan **SIGKILL / proses dihentikan sistem** dapat berarti batas RAM VPS terlampaui; konfirmasi dari log VPS karena sinyal ini juga bisa berasal dari penghentian lain. Pembagian render klip tidak membagi penggabungan akhir, yang tetap memakai VPS utama.

Mulai v1.5.1, penggabungan video dikerjakan bertahap dengan maksimal empat input video tiap proses. Narasi dirangkai berurutan sesuai durasi klip, termasuk jeda hening untuk klip tanpa suara. Decoder video saat penggabungan memakai satu thread per input dan encoder dua thread untuk menekan penggunaan RAM. Tahap tambahan ini dapat menambah waktu encoding, tetapi menghindari membuka seluruh klip dalam satu graph besar. Pilihan resolusi dan FPS tetap mengikuti ekspor.

Jika ekspor lama gagal, buka ulang aplikasi setelah pekerjaan aktif selesai, lalu **Ekspor ulang**. Aset gambar/video/narasi tetap tersedia; tidak perlu melakukan generate AI lagi. Bila RAM VPS masih terbatas, gunakan 1080p 30 FPS atau pilih VPS utama yang RAM-nya lebih besar.
