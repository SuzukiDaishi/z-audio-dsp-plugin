use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use flate2::write::GzEncoder;
use flate2::Compression;

fn main() -> nih_plug_xtask::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("bundle-webclap") {
        bundle_webclap(&args[1..])?;
        Ok(())
    } else {
        nih_plug_xtask::main()
    }
}

fn bundle_webclap(args: &[String]) -> Result<()> {
    nih_plug_xtask::chdir_workspace_root()?;

    let release = args.iter().any(|arg| arg == "--release");
    let profile = if release { "release" } else { "debug" };
    let mut build = Command::new("cargo");
    build
        .arg("build")
        .arg("--target")
        .arg("wasm32-unknown-unknown")
        .arg("-p")
        .arg("z-audio-webclap")
        .arg("-p")
        .arg("z-audio-webclap-eq");
    if release {
        build.arg("--release");
    }

    let status = build
        .status()
        .context("could not run cargo build for WebCLAP")?;
    if !status.success() {
        bail!("WebCLAP wasm build failed");
    }

    let target_dir = Path::new("target").join("webclap");
    fs::create_dir_all(&target_dir).context("could not create target/webclap")?;

    bundle_one_webclap(
        &target_dir,
        "z-audio-simple-synth.wclap",
        Path::new("crates/z-audio-webclap"),
        Path::new("target/wasm32-unknown-unknown")
            .join(profile)
            .join("z_audio_webclap.wasm"),
    )?;
    bundle_one_webclap(
        &target_dir,
        "z-audio-simple-eq.wclap",
        Path::new("crates/z-audio-webclap-eq"),
        Path::new("target/wasm32-unknown-unknown")
            .join(profile)
            .join("z_audio_webclap_eq.wasm"),
    )?;

    Ok(())
}

fn bundle_one_webclap(
    target_dir: &Path,
    bundle_name: &str,
    crate_dir: &Path,
    wasm_path: PathBuf,
) -> Result<()> {
    let bundle_dir = target_dir.join(bundle_name);
    if bundle_dir.exists() {
        fs::remove_dir_all(&bundle_dir)
            .with_context(|| format!("could not remove '{}'", bundle_dir.display()))?;
    }
    fs::create_dir_all(&bundle_dir)
        .with_context(|| format!("could not create '{}'", bundle_dir.display()))?;

    fs::copy(&wasm_path, bundle_dir.join("module.wasm"))
        .with_context(|| format!("could not copy '{}'", wasm_path.display()))?;

    let manifest_path = crate_dir.join("plugin.json");
    let manifest = fs::read_to_string(&manifest_path)
        .with_context(|| format!("could not read '{}'", manifest_path.display()))?;
    let archive_name = format!("{bundle_name}.tar.gz");
    let manifest = rewrite_manifest_for_webclap_archive(&manifest, &archive_name);
    fs::write(bundle_dir.join("plugin.json"), manifest)
        .with_context(|| format!("could not write '{}/plugin.json'", bundle_dir.display()))?;

    copy_dir_recursive(&crate_dir.join("ui"), &bundle_dir.join("ui"))?;
    let archive_path = target_dir.join(archive_name);
    create_webclap_archive(&bundle_dir, &archive_path)?;
    eprintln!("Created WebCLAP bundle at '{}'", bundle_dir.display());
    eprintln!("Created WebCLAP tarball at '{}'", archive_path.display());
    Ok(())
}

fn rewrite_manifest_for_webclap_archive(manifest: &str, archive_name: &str) -> String {
    manifest
        .lines()
        .map(|line| {
            if line.trim_start().starts_with("\"artifact\"") {
                format!("  \"artifact\": \"{archive_name}\",")
            } else if line.trim_start().starts_with("\"format\"") {
                "  \"format\": \"tar.gz\",".to_string()
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

fn create_webclap_archive(bundle_dir: &Path, archive_path: &Path) -> Result<()> {
    if archive_path.exists() {
        fs::remove_file(archive_path)
            .with_context(|| format!("could not remove '{}'", archive_path.display()))?;
    }

    let file = fs::File::create(archive_path)
        .with_context(|| format!("could not create '{}'", archive_path.display()))?;
    let encoder = GzEncoder::new(file, Compression::default());
    let mut archive = tar::Builder::new(encoder);

    archive
        .append_path_with_name(bundle_dir.join("module.wasm"), "module.wasm")
        .with_context(|| format!("could not archive '{}/module.wasm'", bundle_dir.display()))?;
    archive
        .append_path_with_name(bundle_dir.join("plugin.json"), "plugin.json")
        .with_context(|| format!("could not archive '{}/plugin.json'", bundle_dir.display()))?;
    archive
        .append_dir_all("ui", bundle_dir.join("ui"))
        .with_context(|| format!("could not archive '{}/ui'", bundle_dir.display()))?;

    let encoder = archive
        .into_inner()
        .context("could not finish WebCLAP tar archive")?;
    encoder
        .finish()
        .context("could not finish WebCLAP gzip stream")?;
    Ok(())
}

fn copy_dir_recursive(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to).with_context(|| format!("could not create '{}'", to.display()))?;
    for entry in
        fs::read_dir(from).with_context(|| format!("could not read '{}'", from.display()))?
    {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let dest = to.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &dest)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), &dest)
                .with_context(|| format!("could not copy '{}'", entry.path().display()))?;
        }
    }
    Ok(())
}
