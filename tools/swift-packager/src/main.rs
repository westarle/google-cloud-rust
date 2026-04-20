use std::fs;
use std::path::Path;
use std::process::Command;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Building Rust crate for Swift bindings...");

    // 1. Build Rust crate
    let status = Command::new("cargo")
        .args([
            "build",
            "--manifest-path",
            "src/auth-uniffi/Cargo.toml",
            "--release",
            "--target",
            "x86_64-unknown-linux-gnu",
        ])
        .status()?;

    if !status.success() {
        eprintln!("Failed to build Rust crate");
        std::process::exit(1);
    }

    // 2. Create bundle structure
    let bundle_dir = Path::new("target/rust_auth_coreFFI.artifactbundle");
    let variant_dir = bundle_dir.join("linux-x86_64");
    let include_dir = variant_dir.join("include");

    let _ = fs::remove_dir_all(bundle_dir);
    let _ = fs::remove_file("target/rust_auth_coreFFI.artifactbundle.zip");

    fs::create_dir_all(&include_dir)?;

    // 3. Copy static library
    println!("Copying static library...");
    let lib_source =
        Path::new("target/x86_64-unknown-linux-gnu/release/libauth_uniffi.a");
    let lib_dest = variant_dir.join("libauth_uniffi.a");
    fs::copy(lib_source, lib_dest)?;

    // 4. Create dummy headers and module map for prototype
    println!("Creating dummy headers and module map for prototype...");
    fs::write(include_dir.join("auth_uniffi.h"), "// Dummy header\n")?;
    fs::write(
        include_dir.join("module.modulemap"),
        "module auth_uniffi {\n  header \"auth_uniffi.h\"\n  export *\n}\n",
    )?;

    // 5. Create info.json
    println!("Creating info.json...");
    let info_json_content = r#"{
  "schemaVersion": "1.0",
  "artifacts": {
    "auth_uniffi": {
      "version": "0.1.0",
      "type": "staticLibrary",
      "variants": [
        {
          "path": "linux-x86_64/libauth_uniffi.a",
          "supportedTriples": ["x86_64-pc-linux-gnu", "x86_64-unknown-linux-gnu"]
        }
      ],
      "staticLibraryMetadata": {
        "headerPaths": ["linux-x86_64/include"],
        "moduleMapPath": "linux-x86_64/include/module.modulemap"
      }
    }
  }
}"#;

    fs::write(bundle_dir.join("info.json"), info_json_content)?;

    // 6. Zip the bundle
    println!("Zipping artifact bundle...");
    let zip_status = Command::new("zip")
        .current_dir("target")
        .args([
            "-r",
            "rust_auth_coreFFI.artifactbundle.zip",
            "rust_auth_coreFFI.artifactbundle",
        ])
        .status()?;

    if !zip_status.success() {
        eprintln!("Failed to zip artifact bundle");
        std::process::exit(1);
    }

    println!("Artifact bundle created and zipped successfully at target/rust_auth_coreFFI.artifactbundle.zip");

    Ok(())
}
