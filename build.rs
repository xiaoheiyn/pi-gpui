fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    println!("cargo:rerun-if-changed=resources/windows.rc");
    println!("cargo:rerun-if-changed=resources/AppIcon.ico");
    embed_resource::compile_for(
        "resources/windows.rc",
        ["pi-gpui"],
        [
            concat!(
                "APP_VERSION=",
                env!("CARGO_PKG_VERSION_MAJOR"),
                ",",
                env!("CARGO_PKG_VERSION_MINOR"),
                ",",
                env!("CARGO_PKG_VERSION_PATCH"),
                ",0"
            ),
            concat!("APP_VERSION_TEXT=\"", env!("CARGO_PKG_VERSION"), "\""),
        ],
    )
    .manifest_required()
    .expect("embed Pi GPUI's Windows icon and version resources");
}
