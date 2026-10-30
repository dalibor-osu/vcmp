fn main() {
    // Debug info lets tests find elements by id and type (ElementHandle API); release
    // builds leave it out.
    let debug_build = std::env::var("PROFILE").is_ok_and(|profile| profile == "debug");
    let config = slint_build::CompilerConfiguration::new().with_debug_info(debug_build);
    slint_build::compile_with_config("ui/app-window.slint", config)
        .expect("Slint UI failed to compile");

    // On Windows the manifest asks for version 6 of the common controls: the title
    // bar's subclassing needs `GetWindowSubclass`, which only version 6 exports, and
    // without it Windows refuses to start VCMP. It goes into every executable, tests
    // and examples included, as they can open the window too.
    if std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows") {
        let folder = format!("{}/windows", env!("CARGO_MANIFEST_DIR"));
        println!("cargo:rerun-if-changed=windows");
        embed_resource::compile_for_everything(
            format!("{folder}/vcmp.rc"),
            embed_resource::ParamsIncludeDirs([folder.as_str()]),
        )
        .manifest_required()
        .expect("the Windows manifest failed to compile");
    }
}
