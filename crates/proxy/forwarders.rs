// The proxy's DLL name and its 17 export forwarders, shared by build.rs and the tests.

/// The DLL the proxy stands in for. `[lib] name` in Cargo.toml builds the proxy under this name, and a
/// unit test checks that the two agree.
const DLL_NAME: &str = "version";

/// Every export of the System32 version.dll, in name order.
const EXPORTS: [&str; 17] = [
    "GetFileVersionInfoA",
    "GetFileVersionInfoByHandle",
    "GetFileVersionInfoExA",
    "GetFileVersionInfoExW",
    "GetFileVersionInfoSizeA",
    "GetFileVersionInfoSizeExA",
    "GetFileVersionInfoSizeExW",
    "GetFileVersionInfoSizeW",
    "GetFileVersionInfoW",
    "VerFindFileA",
    "VerFindFileW",
    "VerInstallFileA",
    "VerInstallFileW",
    "VerLanguageNameA",
    "VerLanguageNameW",
    "VerQueryValueA",
    "VerQueryValueW",
];

/// Each export name with its forwarder string. A bare `version` module name would name the proxy
/// itself, and the `GLOBALROOT` path to the System32 copy does not depend on the drive letter.
fn forwarders() -> impl Iterator<Item = (&'static str, String)> {
    EXPORTS.into_iter().map(|name| {
        (name, format!(r"\\.\GLOBALROOT\SystemRoot\System32\{DLL_NAME}.{name}"))
    })
}
