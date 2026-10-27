The release build copies the `kara` CLI here (`npm run bundle-cli`, run
automatically before `tauri build`), and the app ships this folder as a
resource. This file keeps the folder present so the resource glob always
matches in dev builds; the binaries themselves are not committed.
