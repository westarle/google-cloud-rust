use std::fs;
use std::path::Path;
use std::process::Command;
use rust_auth_core_builder::GeneratedPaths;

use crate::common::map_swift_to_rust_target;

pub fn generate_xcframework(targets: &[String], paths: &GeneratedPaths) -> Result<(), Box<dyn std::error::Error>> {
    let mut macos_binaries = Vec::new();

    for target in targets {
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
    }

    Ok(())
}
