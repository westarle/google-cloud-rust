use clap::Parser;
use std::fs;
use std::io::{Read, Write, Seek};
use std::path::Path;
use std::process::Command;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    /// Target triples to build for
    #[arg(long, default_value = "x86_64-unknown-linux-gnu")]
    target: Vec<String>,
}

fn map_swift_to_rust_target(swift_target: &str) -> &str {
    match swift_target {
        "x86_64-swift-linux-musl" => "x86_64-unknown-linux-musl",
        "aarch64-swift-linux-musl" => "aarch64-unknown-linux-musl",
        "x86_64-unknown-windows-msvc" => "x86_64-pc-windows-msvc",
        "aarch64-unknown-windows-msvc" => "aarch64-pc-windows-msvc",
        "wasm32-unknown-wasi" => "wasm32-wasip1",
        _ => swift_target,
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    println!("Building Rust crate for Swift bindings for targets {:?}...", cli.target);

    let paths = rust_auth_core_builder::build_bindings("swift")?;
    let version = paths.version.clone();
    println!("Using version {} received from library API", version);

    // Create directory for generated Swift files in the package
    let package_src_dir = Path::new("target/GeneratedPackage/Sources/GoogleCloudAuthInternal");
    fs::create_dir_all(package_src_dir)?;

    // Copy generated Swift file
    fs::copy(&paths.swift_file, package_src_dir.join("rust_auth_core.swift"))?;

    let bundle_dir = Path::new("target/rust_auth_coreFFI.artifactbundle");
    let _ = fs::remove_dir_all(bundle_dir);
    let _ = fs::remove_file("target/rust_auth_coreFFI.artifactbundle.zip");

    let mut variants_json = String::new();

    for target in &cli.target {
        let rust_target = map_swift_to_rust_target(target);
        println!("Building static library for {} (Rust target: {})...", target, rust_target);
        let status = Command::new("cargo")
            .args([
                "build",
                "--package",
                "rust_auth_core",
                "--release",
                "--target",
                rust_target,
                "--target-dir",
                "target",
            ])
            .status()?;

        if !status.success() {
            eprintln!("Failed to build static library for target {}", target);
            std::process::exit(1);
        }

        // Create bundle structure for this variant
        let variant_dir = bundle_dir.join(target);
        let include_dir = variant_dir.join("include");
        fs::create_dir_all(&include_dir)?;

        // Copy static library
        println!("Copying static library for {}...", target);
        let lib_source = Path::new("target")
            .join(rust_target)
            .join("release/librust_auth_core.a");
        let lib_dest = variant_dir.join("librust_auth_coreFFI.a");
        fs::copy(lib_source, lib_dest)?;

        // Copy generated headers and module map to variant include directory
        println!("Copying generated headers and module map to variant directory...");
        fs::copy(&paths.header_file, include_dir.join("rust_auth_coreFFI.h"))?;
        fs::copy(&paths.module_map, include_dir.join("module.modulemap"))?;

        // Append to variants JSON with staticLibraryMetadata INSIDE the variant
        if !variants_json.is_empty() {
            variants_json.push_str(",\n");
        }
        variants_json.push_str(&format!(r#"        {{
          "path": "{}/librust_auth_coreFFI.a",
          "supportedTriples": ["{}"],
          "staticLibraryMetadata": {{
            "headerPaths": ["{}/include"],
            "moduleMapPath": "{}/include/module.modulemap"
          }}
        }}"#, target, target, target, target));
    }

    // 5. Create info.json
    println!("Creating info.json...");
    let info_json_content = format!(r#"{{
  "schemaVersion": "1.0",
  "artifacts": {{
    "rust_auth_coreFFI": {{
      "version": "{}",
      "type": "staticLibrary",
      "variants": [
{}
      ]
    }}
  }}
}}"#, version, variants_json);

    fs::write(bundle_dir.join("info.json"), info_json_content)?;

    // 6. Zip the bundle using zip crate
    println!("Zipping artifact bundle using zip crate...");
    let zip_file = fs::File::create("target/rust_auth_coreFFI.artifactbundle.zip")?;
    let mut zip = zip::ZipWriter::new(zip_file);
    let options = zip::write::FileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    // Helper to add file to zip
    fn add_file_to_zip<W: Write + Seek>(
        zip: &mut zip::ZipWriter<W>,
        path_in_zip: &str,
        file_path: &Path,
        options: zip::write::FileOptions,
    ) -> Result<(), Box<dyn std::error::Error>> {
        zip.start_file(path_in_zip, options)?;
        let mut f = fs::File::open(file_path)?;
        let mut buffer = Vec::new();
        f.read_to_end(&mut buffer)?;
        zip.write_all(&buffer)?;
        Ok(())
    }

    add_file_to_zip(&mut zip, "rust_auth_coreFFI.artifactbundle/info.json", &bundle_dir.join("info.json"), options)?;
    
    for target in &cli.target {
        let variant_dir = bundle_dir.join(target);
        add_file_to_zip(&mut zip, &format!("rust_auth_coreFFI.artifactbundle/{}/librust_auth_coreFFI.a", target), &variant_dir.join("librust_auth_coreFFI.a"), options)?;
        add_file_to_zip(&mut zip, &format!("rust_auth_coreFFI.artifactbundle/{}/include/rust_auth_coreFFI.h", target), &variant_dir.join("include/rust_auth_coreFFI.h"), options)?;
        add_file_to_zip(&mut zip, &format!("rust_auth_coreFFI.artifactbundle/{}/include/module.modulemap", target), &variant_dir.join("include/module.modulemap"), options)?;
    }

    zip.finish()?;

    println!("Artifact bundle created and zipped successfully at target/rust_auth_coreFFI.artifactbundle.zip");

    Ok(())
}
