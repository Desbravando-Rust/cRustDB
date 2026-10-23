use ico_builder::IcoBuilder;
use image::imageops::FilterType;
use std::error::Error;
use std::fs;
use std::path::Path;

const LINUX_SIZES: [u32; 5] = [32, 64, 128, 256, 512];
const WINDOWS_SIZES: [u32; 5] = [16, 24, 32, 48, 256];
const MAC_ICONS: [(&str, u32); 10] = [
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

fn main() -> Result<(), Box<dyn Error>> {
    // Look for the original file in the crustdb Workspace root
    let input_path = Path::new("./assets/icons/original.png");

    if !generate(input_path, Path::new("./assets/icons"))? {
        println!(
            "❌ Error: The file '{}' was not found in the project root.",
            input_path.display()
        );
        println!("Please place a 1024x1024 PNG in the repository root with this name.");
        return Ok(());
    }

    println!("\n🎉 Icons successfully generated in the './assets/' folder!");
    Ok(())
}

/// Generates Linux, Windows and macOS icons under `out_root`.
/// Returns `Ok(false)` when `input` does not exist.
fn generate(input: &Path, out_root: &Path) -> Result<bool, Box<dyn Error>> {
    if !input.exists() {
        return Ok(false);
    }

    println!("⏳ Loading and processing original image...");
    let img = image::open(input)?;

    let linux_folder = out_root.join("linux");
    let windows_folder = out_root.join("windows");
    let mac_folder = out_root.join("mac/icon.iconset");

    println!("📦 Generating individual icons for Linux...");
    fs::create_dir_all(&linux_folder)?;
    for size in LINUX_SIZES {
        img.resize_exact(size, size, FilterType::Lanczos3)
            .save(linux_folder.join(format!("{size}x{size}.png")))?;
    }

    println!("📦 Generating 'icon.ico' file for Windows...");
    fs::create_dir_all(&windows_folder)?;
    let mut builder = IcoBuilder::default();
    for size in WINDOWS_SIZES {
        let temp_path = windows_folder.join(format!("temp_{size}.png"));
        img.resize_exact(size, size, FilterType::Lanczos3)
            .save(&temp_path)?;
        builder.add_source_file(&temp_path);
    }
    builder.build_file(windows_folder.join("icon.ico"))?;
    for size in WINDOWS_SIZES {
        fs::remove_file(windows_folder.join(format!("temp_{size}.png")))?;
    }

    println!("📦 Generating 'icon.iconset' structure for macOS...");
    fs::create_dir_all(&mac_folder)?;
    for (name, size) in MAC_ICONS {
        img.resize_exact(size, size, FilterType::Lanczos3)
            .save(mac_folder.join(name))?;
    }

    Ok(true)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("original.png");
        image::RgbaImage::new(64, 64).save(&input).unwrap();
        (dir, input)
    }

    #[test]
    fn generate_returns_false_when_input_missing() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out");
        let done = generate(&dir.path().join("nope.png"), &out).unwrap();
        assert!(!done);
        assert!(!out.exists());
    }

    #[test]
    fn generate_writes_linux_pngs_with_exact_sizes() {
        let (dir, input) = fixture();
        assert!(generate(&input, dir.path()).unwrap());
        for size in LINUX_SIZES {
            let p = dir.path().join(format!("linux/{size}x{size}.png"));
            assert_eq!(image::image_dimensions(p).unwrap(), (size, size));
        }
    }

    #[test]
    fn generate_writes_windows_ico_and_removes_temps() {
        let (dir, input) = fixture();
        assert!(generate(&input, dir.path()).unwrap());
        let win = dir.path().join("windows");
        assert!(fs::metadata(win.join("icon.ico")).unwrap().len() > 0);
        for size in WINDOWS_SIZES {
            assert!(!win.join(format!("temp_{size}.png")).exists());
        }
    }

    #[test]
    fn generate_writes_full_macos_iconset() {
        let (dir, input) = fixture();
        assert!(generate(&input, dir.path()).unwrap());
        for (name, size) in MAC_ICONS {
            let p = dir.path().join("mac/icon.iconset").join(name);
            assert_eq!(image::image_dimensions(p).unwrap(), (size, size));
        }
    }
}
