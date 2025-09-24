fn main() {
    // Generate client code for river-status protocol only (used for logging).
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let input = std::path::Path::new("protocol/river-status-unstable-v1.xml");
    let output = std::path::Path::new(&out_dir).join("river-status-unstable-v1.rs");

    wayland_scanner::generate_code(
        input,
        output,
        wayland_scanner::Side::Client,
    );
}

