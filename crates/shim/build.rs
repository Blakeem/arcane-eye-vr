//! Compiles the vendored SafetyHook and Zydis amalgamation, taken from the SafetyHook v0.7.0 release
//! asset `safetyhook-amalgamated-zydis.zip` (sha256 505d4c07ec1c5b94a17f3906ca86afbe1264e738d8becaa244866694c6200c2c),
//! and the shim's own C ABI sources in `csrc/`.
//! MSVC ignores an unknown `/std` switch with only a warning, so `/options:strict` turns it into an error.

const VENDOR_DIR: &str = "vendor/safetyhook";
const SOURCE_DIR: &str = "csrc";

fn main() {
    println!("cargo::rerun-if-changed={VENDOR_DIR}");
    println!("cargo::rerun-if-changed={SOURCE_DIR}");

    cc::Build::new()
        .file(format!("{VENDOR_DIR}/Zydis.c"))
        .include(VENDOR_DIR)
        .flag("/options:strict")
        .compile("zydis");

    cc::Build::new()
        .cpp(true)
        .std("c++23preview")
        .file(format!("{VENDOR_DIR}/safetyhook.cpp"))
        .file(format!("{SOURCE_DIR}/mid_hook.cpp"))
        .include(VENDOR_DIR)
        .flag("/options:strict")
        .flag("/EHsc")
        .flag("/Zc:__cplusplus")
        .compile("safetyhook");

    // `__try` is C structured exception handling, so the guarded copy is a C file.
    cc::Build::new()
        .file(format!("{SOURCE_DIR}/guarded_copy.c"))
        .flag("/options:strict")
        .compile("guarded_copy");
}
