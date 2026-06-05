fn main() {
    if std::env::var("CARGO_FEATURE_CAPI").is_err() {
        return;
    }

    println!("cargo:rerun-if-changed=src/ffi.rs");
    generate_header();
}

#[cfg(feature = "capi")]
fn generate_header() {
    let crate_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set");

    cbindgen::Builder::new()
        .with_crate(crate_dir)
        .with_language(cbindgen::Language::C)
        .with_include_guard("AUDIOSCAN_H")
        .with_header(
            "/*\n\
             * Analyze audio files with audioscan.\n\
             *\n\
             * audioscan_analyze_json returns a newly allocated JSON string on success and NULL\n\
             * on hard errors, invalid input, invalid UTF-8 paths, or caught Rust panics. The\n\
             * caller owns successful return values and must free them with\n\
             * audioscan_string_free. audioscan_version returns static storage and must not be\n\
             * freed.\n\
             */",
        )
        .generate()
        .expect("generate audioscan C header")
        .write_to_file("include/audioscan.h");
}

#[cfg(not(feature = "capi"))]
fn generate_header() {}
