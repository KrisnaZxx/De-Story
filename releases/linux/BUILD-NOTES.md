# De Story 1.1.0 — Linux desktop build

Build pengujian **x86_64 Debian 13**, dengan antarmuka GUI. Paket ini membutuhkan **glibc 2.39+**; belum diuji langsung pada desktop pengguna. Gunakan `.deb` melalui pengelola paket grafis atau jadikan `.AppImage` executable melalui Properties → Permissions.

Ubuntu 22.04 dan Debian 12 membutuhkan paket yang dibuat pada lingkungan lebih lama. Workflow rilis repository menggunakan Ubuntu 22.04 untuk keperluan tersebut; kompatibilitas desktop tetap perlu diuji sebelum publikasi.

Pemeriksaan yang lulus pada Linux: build native dan paket `.deb`/`.AppImage`, typecheck, pemeriksaan editor/model, akses media beserta batas path, rendering data caption ASS, pemulihan edit/database, serta kontrak HTTP Vidgen memakai server simulasi.

Whisper lokal disertakan sebagai binary Linux. FFmpeg, OpenSSH, keyring desktop, pustaka WebKitGTK/Chromium, dan codec multimedia merupakan dependensi sistem. Kunci tersimpan pada keyring pengguna Linux; key Windows perlu diisi ulang saat pindah OS.

Belum dilakukan pengujian GUI desktop maupun generasi memakai akun/API key sungguhan. Periksa caption dan hasil generasi sebelum dipublikasikan.
