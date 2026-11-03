fn main() {
    println!("cargo:rerun-if-env-changed=SLINT_FONT_SIZES");
    if std::env::var_os("SLINT_FONT_SIZES").is_none() {
        std::env::set_var(
            "SLINT_FONT_SIZES",
            "10,11,12,12.5,13,14,15,16,18,19.2,20,22,24,26,28,30,32,34,36,38,40,42,44",
        );
    }
    let config = slint_build::CompilerConfiguration::new()
        .with_debug_info(std::env::var("PROFILE").as_deref() != Ok("release"))
        .embed_resources(slint_build::EmbedResourcesKind::EmbedForSoftwareRenderer);
    slint_build::compile_with_config("ui/app.slint", config).expect("failed to compile Umbra UI");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("icons/icon.ico")
            .compile()
            .expect("failed to compile Windows application resources");
    }
}
