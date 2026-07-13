use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use flate2::write::GzEncoder;
use flate2::Compression;

mod sampler_bank;
mod vcsl_piano;

struct WebClapBundle {
    package: &'static str,
    bundle_name: &'static str,
    crate_dir: &'static str,
    wasm_file: &'static str,
}

const WEBCLAP_BUNDLES: &[WebClapBundle] = &[
    WebClapBundle {
        package: "z-audio-webclap",
        bundle_name: "z-audio-simple-synth.wclap",
        crate_dir: "crates/z-audio-webclap",
        wasm_file: "z_audio_webclap.wasm",
    },
    WebClapBundle {
        package: "z-audio-webclap-eq",
        bundle_name: "z-audio-simple-eq.wclap",
        crate_dir: "crates/z-audio-webclap-eq",
        wasm_file: "z_audio_webclap_eq.wasm",
    },
    WebClapBundle {
        package: "z-audio-webclap-piano",
        bundle_name: "z-audio-formula-piano.wclap",
        crate_dir: "crates/z-audio-webclap-piano",
        wasm_file: "z_audio_webclap_piano.wasm",
    },
    WebClapBundle {
        package: "z-audio-webclap-vcsl-piano",
        bundle_name: "z-audio-vcsl-piano.wclap",
        crate_dir: "crates/z-audio-webclap-vcsl-piano",
        wasm_file: "z_audio_webclap_vcsl_piano.wasm",
    },
    WebClapBundle {
        package: "z-audio-webclap-sampler",
        bundle_name: "z-audio-sampler.wclap",
        crate_dir: "crates/z-audio-webclap-sampler",
        wasm_file: "z_audio_webclap_sampler.wasm",
    },
    WebClapBundle {
        package: "z-audio-webclap-granular",
        bundle_name: "z-audio-granular.wclap",
        crate_dir: "crates/z-audio-webclap-granular",
        wasm_file: "z_audio_webclap_granular.wasm",
    },
    WebClapBundle {
        package: "z-audio-webclap-wavetable",
        bundle_name: "z-audio-wavetable.wclap",
        crate_dir: "crates/z-audio-webclap-wavetable",
        wasm_file: "z_audio_webclap_wavetable.wasm",
    },
    WebClapBundle {
        package: "z-audio-webclap-drums",
        bundle_name: "z-audio-formula-drums.wclap",
        crate_dir: "crates/z-audio-webclap-drums",
        wasm_file: "z_audio_webclap_drums.wasm",
    },
    WebClapBundle {
        package: "z-audio-webclap-reverb",
        bundle_name: "z-audio-parametric-reverb.wclap",
        crate_dir: "crates/z-audio-webclap-reverb",
        wasm_file: "z_audio_webclap_reverb.wasm",
    },
    WebClapBundle {
        package: "z-audio-webclap-diffuser",
        bundle_name: "z-audio-diffuser.wclap",
        crate_dir: "crates/z-audio-webclap-diffuser",
        wasm_file: "z_audio_webclap_diffuser.wasm",
    },
    WebClapBundle {
        package: "z-audio-webclap-limiter",
        bundle_name: "z-audio-limiter.wclap",
        crate_dir: "crates/z-audio-webclap-limiter",
        wasm_file: "z_audio_webclap_limiter.wasm",
    },
    WebClapBundle {
        package: "z-audio-webclap-compressor",
        bundle_name: "z-audio-compressor.wclap",
        crate_dir: "crates/z-audio-webclap-compressor",
        wasm_file: "z_audio_webclap_compressor.wasm",
    },
    WebClapBundle {
        package: "z-audio-webclap-ringmod",
        bundle_name: "z-audio-ring-mod.wclap",
        crate_dir: "crates/z-audio-webclap-ringmod",
        wasm_file: "z_audio_webclap_ringmod.wasm",
    },
    WebClapBundle {
        package: "z-audio-webclap-distortion",
        bundle_name: "z-audio-distortion.wclap",
        crate_dir: "crates/z-audio-webclap-distortion",
        wasm_file: "z_audio_webclap_distortion.wasm",
    },
    WebClapBundle {
        package: "z-audio-webclap-saturator",
        bundle_name: "z-audio-saturator.wclap",
        crate_dir: "crates/z-audio-webclap-saturator",
        wasm_file: "z_audio_webclap_saturator.wasm",
    },
    WebClapBundle {
        package: "z-audio-webclap-bitcrusher",
        bundle_name: "z-audio-bitcrusher.wclap",
        crate_dir: "crates/z-audio-webclap-bitcrusher",
        wasm_file: "z_audio_webclap_bitcrusher.wasm",
    },
    WebClapBundle {
        package: "z-audio-webclap-delay",
        bundle_name: "z-audio-delay.wclap",
        crate_dir: "crates/z-audio-webclap-delay",
        wasm_file: "z_audio_webclap_delay.wasm",
    },
    WebClapBundle {
        package: "z-audio-webclap-chorus",
        bundle_name: "z-audio-chorus.wclap",
        crate_dir: "crates/z-audio-webclap-chorus",
        wasm_file: "z_audio_webclap_chorus.wasm",
    },
    WebClapBundle {
        package: "z-audio-webclap-flanger",
        bundle_name: "z-audio-flanger.wclap",
        crate_dir: "crates/z-audio-webclap-flanger",
        wasm_file: "z_audio_webclap_flanger.wasm",
    },
    WebClapBundle {
        package: "z-audio-webclap-phaser",
        bundle_name: "z-audio-phaser.wclap",
        crate_dir: "crates/z-audio-webclap-phaser",
        wasm_file: "z_audio_webclap_phaser.wasm",
    },
    WebClapBundle {
        package: "z-audio-webclap-tremolo",
        bundle_name: "z-audio-tremolo.wclap",
        crate_dir: "crates/z-audio-webclap-tremolo",
        wasm_file: "z_audio_webclap_tremolo.wasm",
    },
    WebClapBundle {
        package: "z-audio-webclap-gate",
        bundle_name: "z-audio-gate.wclap",
        crate_dir: "crates/z-audio-webclap-gate",
        wasm_file: "z_audio_webclap_gate.wasm",
    },
    WebClapBundle {
        package: "z-audio-webclap-hyperdim",
        bundle_name: "z-audio-hyperdim.wclap",
        crate_dir: "crates/z-audio-webclap-hyperdim",
        wasm_file: "z_audio_webclap_hyperdim.wasm",
    },
    WebClapBundle {
        package: "z-audio-webclap-ott",
        bundle_name: "z-audio-ott.wclap",
        crate_dir: "crates/z-audio-webclap-ott",
        wasm_file: "z_audio_webclap_ott.wasm",
    },
    WebClapBundle {
        package: "z-audio-webclap-vocoder",
        bundle_name: "z-audio-vocoder.wclap",
        crate_dir: "crates/z-audio-webclap-vocoder",
        wasm_file: "z_audio_webclap_vocoder.wasm",
    },
];

fn main() -> nih_plug_xtask::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("bundle-webclap") {
        bundle_webclap(&args[1..])?;
        Ok(())
    } else if args.first().map(String::as_str) == Some("prepare-vcsl-piano") {
        vcsl_piano::run(&args[1..])?;
        Ok(())
    } else if args.first().map(String::as_str) == Some("prepare-sampler-bank") {
        sampler_bank::run(&args[1..])?;
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
        .arg("wasm32-unknown-unknown");
    for bundle in WEBCLAP_BUNDLES {
        build.arg("-p").arg(bundle.package);
    }
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

    for bundle in WEBCLAP_BUNDLES {
        bundle_one_webclap(
            &target_dir,
            bundle.bundle_name,
            Path::new(bundle.crate_dir),
            Path::new("target/wasm32-unknown-unknown")
                .join(profile)
                .join(bundle.wasm_file),
        )?;
    }

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

    let ui_dir = crate_dir.join("ui");
    if ui_dir.is_dir() {
        copy_dir_recursive(&ui_dir, &bundle_dir.join("ui"))?;
    }
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
    let ui_dir = bundle_dir.join("ui");
    if ui_dir.is_dir() {
        archive
            .append_dir_all("ui", ui_dir)
            .with_context(|| format!("could not archive '{}/ui'", bundle_dir.display()))?;
    }

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
