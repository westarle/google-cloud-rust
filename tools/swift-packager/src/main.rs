use clap::Parser;
use std::fs;
use std::io::{Read, Write, Seek};
use std::path::Path;
use std::process::Command;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    /// Target triples to build for
    #[arg(long, default_value = "x86_64-unknown-linux-gnu", value_delimiter = ',')]
    target: Vec<String>,
}

fn map_swift_to_rust_target(swift_target: &str) -> &str {
    match swift_target {
        "x86_64-swift-linux-musl" => "x86_64-unknown-linux-musl",
        "aarch64-swift-linux-musl" => "aarch64-unknown-linux-musl",
        "x86_64-unknown-windows-msvc" => "x86_64-pc-windows-msvc",
        "aarch64-unknown-windows-msvc" => "aarch64-pc-windows-msvc",
        "wasm32-unknown-wasi" => "wasm32-wasip1",
        "x86_64-apple-macosx" => "x86_64-apple-darwin",
        "arm64-apple-macosx" => "aarch64-apple-darwin",
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
    let mut macos_binaries = Vec::new();

    for target in &cli.target {
        let rust_target = map_swift_to_rust_target(target);
        
        if rust_target.contains("apple-darwin") {
            println!("Building dynamic library for macOS {}...", target);
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
                eprintln!("Failed to build dynamic library for target {}", target);
                std::process::exit(1);
            }
            
            let dylib_path = Path::new("target")
                .join(rust_target)
                .join("release/librust_auth_core.dylib");
            macos_binaries.push((target.clone(), dylib_path));
        } else {
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
    }

    if !macos_binaries.is_empty() {
        println!("Creating XCFramework for macOS...");

        let xcframework_path = Path::new("target/rust_auth_coreFFI.xcframework");
        let _ = fs::remove_dir_all(xcframework_path);

        let temp_dylib = Path::new("target/librust_auth_coreFFI_temp.dylib");
        let _ = fs::remove_file(temp_dylib);

        if macos_binaries.len() > 1 {
            println!("Combining architectures using lipo...");
            let mut lipo_args = vec!["-create", "-output"];
            lipo_args.push(temp_dylib.to_str().unwrap());

            for (_, path) in &macos_binaries {
                lipo_args.push(path.to_str().unwrap());
            }

            let status = Command::new("lipo")
                .args(&lipo_args)
                .status()?;

            if !status.success() {
                eprintln!("Failed to run lipo to combine architectures");
                std::process::exit(1);
            }
        } else if macos_binaries.len() == 1 {
            println!("Copying single macOS binary...");
            fs::copy(&macos_binaries[0].1, temp_dylib)?;
        }

        // Create temporary directory for headers
        let temp_headers_dir = Path::new("target/Headers_temp");
        let _ = fs::remove_dir_all(temp_headers_dir);
        fs::create_dir_all(temp_headers_dir)?;
        
        fs::copy(&paths.header_file, temp_headers_dir.join("rust_auth_coreFFI.h"))?;
        
        let module_map_content = "module rust_auth_coreFFI {\n    header \"rust_auth_coreFFI.h\"\n    export *\n}\n";
        fs::write(temp_headers_dir.join("module.modulemap"), module_map_content)?;

        // Use xcodebuild to create XCFramework
        println!("Creating XCFramework using xcodebuild...");
        let status = Command::new("xcodebuild")
            .args([
                "-create-xcframework",
                "-library",
                temp_dylib.to_str().unwrap(),
                "-headers",
                temp_headers_dir.to_str().unwrap(),
                "-output",
                xcframework_path.to_str().unwrap(),
            ])
            .status()?;

        if !status.success() {
            eprintln!("Failed to create XCFramework using xcodebuild");
            std::process::exit(1);
        }

        println!("XCFramework created successfully at target/rust_auth_coreFFI.xcframework");
        println!("XCFramework created successfully at target/rust_auth_coreFFI.xcframework");
    }

    if !variants_json.is_empty() {
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
            if !target.contains("apple-darwin") {
                let variant_dir = bundle_dir.join(target);
                add_file_to_zip(&mut zip, &format!("rust_auth_coreFFI.artifactbundle/{}/librust_auth_coreFFI.a", target), &variant_dir.join("librust_auth_coreFFI.a"), options)?;
                add_file_to_zip(&mut zip, &format!("rust_auth_coreFFI.artifactbundle/{}/include/rust_auth_coreFFI.h", target), &variant_dir.join("include/rust_auth_coreFFI.h"), options)?;
                add_file_to_zip(&mut zip, &format!("rust_auth_coreFFI.artifactbundle/{}/include/module.modulemap", target), &variant_dir.join("include/module.modulemap"), options)?;
            }
        }

        zip.finish()?;

        println!("Artifact bundle created and zipped successfully at target/rust_auth_coreFFI.artifactbundle.zip");
    }

    Ok(())
}
