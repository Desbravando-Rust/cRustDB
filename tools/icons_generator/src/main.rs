use ico_builder::IcoBuilder;
use image::imageops::FilterType;
use std::fs;
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Look for the original file in the crustdb Workspace root
    let input_path = "./assets/icons/original.png";

    if !Path::new(input_path).exists() {
        println!(
            "❌ Error: The file '{}' was not found in the project root.",
            input_path
        );
        println!("Please place a 1024x1024 PNG in the repository root with this name.");
        return Ok(());
    }

    println!("⏳ Loading and processing original image...");
    let img = image::open(input_path)?;

    // Define output paths organized by platform
    let linux_folder = "./assets/icons/linux";
    let windows_folder = "./assets/icons/windows";
    let mac_folder = "./assets/icons/mac/icon.iconset";

    // -------------------------------------------------------------
    // 1. LINUX
    // -------------------------------------------------------------
    println!("📦 Generating individual icons for Linux...");
    fs::create_dir_all(linux_folder)?;
    let linux_sizes = vec![32, 64, 128, 256, 512];
    for size in linux_sizes {
        let resized = img.resize_exact(size, size, FilterType::Lanczos3);
        resized.save(format!("{}/{}x{}.png", linux_folder, size, size))?;
    }

    // -------------------------------------------------------------
    // 2. WINDOWS
    // -------------------------------------------------------------
    println!("📦 Generating 'icon.ico' file for Windows...");
    fs::create_dir_all(windows_folder)?;

    let windows_sizes = vec![16, 24, 32, 48, 256];
    let mut builder = IcoBuilder::default();

    for size in &windows_sizes {
        let temp_path = format!("{}/temp_{}.png", windows_folder, size);
        img.resize_exact(*size, *size, FilterType::Lanczos3)
            .save(&temp_path)?;
        builder.add_source_file(&temp_path);
    }

    builder.build_file(format!("{}/icon.ico", windows_folder))?;

    for size in windows_sizes {
        fs::remove_file(format!("{}/temp_{}.png", windows_folder, size))?;
    }

    // -------------------------------------------------------------
    // 3. MACOS
    // -------------------------------------------------------------
    println!("📦 Generating 'icon.iconset' structure for macOS...");
    fs::create_dir_all(mac_folder)?;

    let mac_settings = vec![
        ("icon_16x16.png", 16),
        ("icon_16x16@2x.png", 32),
        ("icon_32x32.png", 32),
        ("icon_32x32@2x.png", 64),
        ("icon_128x128.png", 128),
        ("icon_128x128@2x.png", 256),
        ("icon_256x256.png", 256),
        ("icon_256x256@2x.png", 512),
        ("icon_512x512.png", 512),
        ("icon_512x512@2x.png", 1024),
    ];

    for (name, size) in mac_settings {
        let resized = img.resize_exact(size, size, FilterType::Lanczos3);
        resized.save(format!("{}/{}", mac_folder, name))?;
    }

    println!("\n🎉 Icons successfully generated in the './assets/' folder!");
    Ok(())
}
