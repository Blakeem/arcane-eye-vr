//! Compiles the Slang shaders in `shaders/` to DXBC with the `slangc` release that
//! `shaders/slangc.toml` pins, and fails the build on any warning.
//!
//! `tools/fetch-slang.ps1` downloads the compiler. `slangc` emits HLSL and calls `D3DCompile` in
//! System32's `d3dcompiler_47.dll` for DXBC, so the build runs on Windows only. `-warnings-as-errors`
//! does not cover the warnings of that second stage, so any warning line `slangc` prints fails the
//! build too.

#![expect(
    clippy::disallowed_methods,
    reason = "Cargo passes OUT_DIR to a build script only through the environment"
)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs, io};

const SHADER_DIR: &str = "../../shaders";
const PIN_FILE: &str = "../../shaders/slangc.toml";
const DOWNLOADS: &str = "../../reference/downloads";

/// One entry point of a shader source and the DXBC file it compiles to in `OUT_DIR`.
struct Shader {
    source: &'static str,
    entry: &'static str,
    profile: &'static str,
    output: &'static str,
}

const SHADERS: [Shader; 2] = [
    Shader {
        source: "layer.slang",
        entry: "vertex_main",
        profile: "vs_5_0",
        output: "layer.vs.dxbc",
    },
    Shader {
        source: "layer.slang",
        entry: "pixel_main",
        profile: "ps_5_0",
        output: "layer.ps.dxbc",
    },
];

#[derive(Debug, thiserror::Error)]
enum BuildError {
    #[error("Cargo set no OUT_DIR for the build script")]
    NoOutDir(#[from] env::VarError),
    #[error("reading {PIN_FILE} failed: {0}")]
    ReadPin(#[from] io::Error),
    #[error("{PIN_FILE} is no TOML table: {0}")]
    PinSyntax(#[from] toml::de::Error),
    #[error("{PIN_FILE} holds no `version` string")]
    NoVersion,
    #[error("{} is missing. Run `pwsh tools/fetch-slang.ps1`, which downloads the pinned compiler.", .0.display())]
    NoCompiler(PathBuf),
    #[error("starting {} failed: {source}", compiler.display())]
    Start { compiler: PathBuf, source: io::Error },
    #[error("slangc rejected {source_file}, entry {entry}: {output}")]
    Rejected { source_file: &'static str, entry: &'static str, output: String },
}

fn main() {
    println!("cargo::rerun-if-changed={SHADER_DIR}");
    if let Err(error) = compile_all() {
        for line in error.to_string().lines() {
            println!("cargo::error={line}");
        }
    }
}

fn compile_all() -> Result<(), BuildError> {
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let compiler = pinned_compiler()?;
    for shader in &SHADERS {
        compile(&compiler, shader, &out_dir)?;
    }
    Ok(())
}

/// The path of the `slangc` that the pin file names, where `tools/fetch-slang.ps1` unpacks it.
fn pinned_compiler() -> Result<PathBuf, BuildError> {
    let table = fs::read_to_string(PIN_FILE)?.parse::<toml::Table>()?;
    let version =
        table.get("version").and_then(toml::Value::as_str).ok_or(BuildError::NoVersion)?;
    let compiler = Path::new(DOWNLOADS).join(format!("slang-{version}")).join("bin/slangc.exe");
    if !compiler.is_file() {
        return Err(BuildError::NoCompiler(compiler));
    }
    Ok(compiler)
}

/// Compiles one entry point. The matrix layout is named because the Slang language default
/// (row-major) and the `slangc` default (column-major) differ. glam stores matrices column-major.
fn compile(compiler: &Path, shader: &Shader, out_dir: &Path) -> Result<(), BuildError> {
    let started = Command::new(compiler)
        .arg(Path::new(SHADER_DIR).join(shader.source))
        .args(["-target", "dxbc", "-profile", shader.profile, "-entry", shader.entry])
        .args(["-O2", "-warnings-as-errors", "all", "-matrix-layout-column-major", "-o"])
        .arg(out_dir.join(shader.output))
        .output();
    let ran = match started {
        Ok(ran) => ran,
        Err(source) => return Err(BuildError::Start { compiler: compiler.to_owned(), source }),
    };
    let mut output = String::from_utf8_lossy(&ran.stderr).into_owned();
    output.push_str(&String::from_utf8_lossy(&ran.stdout));
    if !ran.status.success() || output.contains("warning") || output.contains("error") {
        return Err(BuildError::Rejected {
            source_file: shader.source,
            entry: shader.entry,
            output,
        });
    }
    Ok(())
}
