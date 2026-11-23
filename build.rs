fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/rustcord.ico");
        res.set("ProductName", "Rustcord");
        res.set("FileDescription", "Rustcord - Ultra-Lightweight Rust Discord Client");
        res.set("OriginalFilename", "rustcord.exe");
        res.set("LegalCopyright", "Rustcord");
        if let Err(e) = res.compile() {
            println!("cargo:warning=Failed to compile Windows resources: {e}");
        }
    }
    println!("cargo:rerun-if-changed=assets/rustcord.ico");
    println!("cargo:rerun-if-changed=assets/rustcord_32.rgba");
}
