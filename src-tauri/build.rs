fn main() {
    // The program's icon is built in from icons/. Tauri's build step watches the config files,
    // not these, so without this line a new logo is silently left out and the exe keeps the old
    // one (seen 2026-11-22: icon files replaced, build exit 0, old logo still inside Tasker.exe).
    println!("cargo:rerun-if-changed=icons");
    tauri_build::build()
}
