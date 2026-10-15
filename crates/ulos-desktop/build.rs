fn main() {
    println!("cargo:rerun-if-changed=../../assets/branding/ulos.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("../../assets/branding/ulos.ico");
        resource.set("ProductName", "Ulos");
        resource.set("FileDescription", "Ulos Download Manager");
        resource.set(
            "LegalCopyright",
            "Copyright 2026 Ulos Download Manager contributors.",
        );
        resource.compile().expect("compile Ulos Windows resources");
    }
}
