fn main() {
    println!("cargo:rerun-if-changed=assets/icons/laralah.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icons/laralah.ico");
        res.set("ProductName", "Laralah");
        res.set("FileDescription", "Laralah — Laravel Desktop REPL & Inspector");
        if let Err(err) = res.compile() {
            panic!("failed to compile Windows icon resource: {err}");
        }
    }
}
