fn main() {
    // Debug info lets tests find elements by id and type (ElementHandle API); release
    // builds leave it out.
    let debug_build = std::env::var("PROFILE").is_ok_and(|profile| profile == "debug");
    let config = slint_build::CompilerConfiguration::new().with_debug_info(debug_build);
    slint_build::compile_with_config("ui/app-window.slint", config)
        .expect("Slint UI failed to compile");
}
