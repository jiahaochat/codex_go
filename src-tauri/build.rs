fn main() {
    println!("cargo:rerun-if-env-changed=CODEX_GO_DEFAULT_VLESS_URI");
    tauri_build::build()
}
