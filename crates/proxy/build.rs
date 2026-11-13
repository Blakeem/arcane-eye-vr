//! Links each version.dll export as a PE forwarder to the System32 copy.
//! The loader resolves a forwarder itself, so `DllMain` never calls `LoadLibrary` for the system DLL.

include!("forwarders.rs");

fn main() {
    println!("cargo::rerun-if-changed=forwarders.rs");
    for (name, forwarder) in forwarders() {
        println!("cargo::rustc-cdylib-link-arg=/EXPORT:{name}={forwarder}");
    }
}
