fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        let out = std::process::Command::new("clang")
            .arg("--print-resource-dir")
            .output()
            .expect("clang not found");
        let dir = String::from_utf8(out.stdout).unwrap();
        println!("cargo:rustc-link-search={}/lib/darwin", dir.trim());
        println!("cargo:rustc-link-lib=static=clang_rt.osx");
    }
    tauri_build::build()
}
