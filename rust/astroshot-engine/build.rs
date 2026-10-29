//! Build-time only npm install. The resulting binary never runs npm or downloads tools.
use flate2::{Compression, write::GzEncoder};
use sha2::{Digest, Sha256};
use std::{env, fs, io, path::Path, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=../node-helper/helper.mjs");
    println!("cargo:rerun-if-changed=../node-helper/package.json");
    println!("cargo:rerun-if-changed=../node-helper/package-lock.json");
    let target = env::var("TARGET").unwrap();
    let os = match target.as_str() {
        "aarch64-apple-darwin" | "x86_64-apple-darwin" => "darwin",
        "aarch64-unknown-linux-gnu" | "x86_64-unknown-linux-gnu" | "x86_64-unknown-linux-musl" => {
            "linux"
        }
        "x86_64-pc-windows-msvc" | "aarch64-pc-windows-msvc" => "win32",
        _ => panic!("no Astroshots helper distribution mapping for {target}"),
    };
    // Node chooses native packages using its own architecture and libc, not
    // Rust's target. Include every supported Node runtime for this OS.
    let variants: &[(&str, &str)] = if os == "linux" {
        &[
            ("arm64", "glibc"),
            ("arm64", "musl"),
            ("x64", "glibc"),
            ("x64", "musl"),
        ]
    } else {
        &[("arm64", ""), ("x64", "")]
    };
    let out = std::path::PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let stage = out.join("node-helper-build");
    fs::create_dir_all(&stage).unwrap();
    for name in ["package.json", "package-lock.json", "helper.mjs"] {
        fs::copy(Path::new("../node-helper").join(name), stage.join(name)).unwrap();
    }
    fs::write(stage.join("target"), &target).unwrap();
    install(&stage, os, variants[0].0, variants[0].1);
    let native = out.join("node-helper-native");
    fs::create_dir_all(&native).unwrap();
    for name in ["package.json", "package-lock.json"] {
        fs::copy(stage.join(name), native.join(name)).unwrap();
    }
    for &(cpu, libc) in &variants[1..] {
        install(&native, os, cpu, libc);
        // Common JS is identical under the lockfile. Merge only native package
        // scopes, retaining the two different esbuild dependency trees.
        for scope in [
            "node_modules/@esbuild",
            "node_modules/vite/node_modules/@esbuild",
            "node_modules/@rollup",
        ] {
            copy_tree(&native.join(scope), &stage.join(scope)).unwrap();
        }
    }
    let encoder = GzEncoder::new(Vec::new(), Compression::default());
    let mut archive = tar::Builder::new(encoder);
    append(&mut archive, &stage, &stage).unwrap();
    let bytes = archive.into_inner().unwrap().finish().unwrap();
    let hash = format!("{:x}", Sha256::digest(&bytes));
    fs::write(out.join("node-helper.tar.gz"), bytes).unwrap();
    println!("cargo:rustc-env=ASTROSHOT_HELPER_DIGEST={hash}");
    println!("cargo:rustc-env=ASTROSHOT_HELPER_TARGET={target}");
}

fn install(stage: &Path, os: &str, cpu: &str, libc: &str) {
    let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };
    let output = Command::new(npm)
        .current_dir(stage)
        .args([
            "ci",
            "--ignore-scripts",
            "--no-audit",
            "--no-fund",
            "--include=optional",
            "--workspaces=false",
        ])
        .arg(format!("--os={os}"))
        .arg(format!("--cpu={cpu}"))
        .args(if libc.is_empty() {
            vec![]
        } else {
            vec![format!("--libc={libc}")]
        })
        .output()
        .expect("building the Astroshots helper requires Node >=22.14 and npm >=11 on PATH");
    assert!(
        output.status.success(),
        "helper npm ci failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    // npm may silently omit optional dependencies: verify both esbuild versions and Rollup.
    for base in ["node_modules", "node_modules/vite/node_modules"] {
        assert!(
            stage
                .join(format!("{base}/@esbuild/{os}-{cpu}/package.json"))
                .is_file(),
            "missing target esbuild under {base}"
        );
    }
    let suffix = match os {
        "linux" => {
            if libc == "musl" {
                "-musl"
            } else {
                "-gnu"
            }
        }
        "win32" => "-msvc",
        _ => "",
    };
    assert!(
        stage
            .join(format!(
                "node_modules/@rollup/rollup-{os}-{cpu}{suffix}/package.json"
            ))
            .is_file(),
        "missing target Rollup"
    );
}

fn copy_tree(source: &Path, destination: &Path) -> io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), target)?;
        } else {
            return Err(io::Error::other("unexpected native package symlink"));
        }
    }
    Ok(())
}

fn append<W: io::Write>(archive: &mut tar::Builder<W>, root: &Path, dir: &Path) -> io::Result<()> {
    let mut entries = fs::read_dir(dir)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name();
        // npm command shims are unnecessary, platform-dependent, and may be symlinks.
        if name == ".bin" || name == ".package-lock.json" {
            continue;
        }
        let ty = entry.file_type()?;
        if ty.is_dir() {
            append(archive, root, &path)?;
        } else if ty.is_file() {
            let bytes = fs::read(&path)?;
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(if name == "esbuild" { 0o755 } else { 0o644 });
            header.set_mtime(0);
            header.set_cksum();
            archive.append_data(
                &mut header,
                path.strip_prefix(root).unwrap(),
                bytes.as_slice(),
            )?;
        } else {
            return Err(io::Error::other(format!(
                "unexpected package symlink: {}",
                path.display()
            )));
        }
    }
    Ok(())
}
