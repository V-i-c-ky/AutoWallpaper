use std::env;
use std::path::{Path, PathBuf};

fn env_or(key: &str, default: &str) -> String {
    env::var(key)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn parse_version_u64(version: &str) -> u64 {
    let mut nums = [0u16; 4];
    for (idx, seg) in version.split('.').take(4).enumerate() {
        nums[idx] = seg.parse::<u16>().unwrap_or(0);
    }
    ((nums[0] as u64) << 48) | ((nums[1] as u64) << 32) | ((nums[2] as u64) << 16) | nums[3] as u64
}

fn icon_path() -> PathBuf {
    let configured = env::var("AW_ICON_PATH").ok().filter(|s| !s.trim().is_empty());
    if let Some(p) = configured {
        return PathBuf::from(p);
    }
    Path::new("data").join("app.ico")
}

fn main() {
    #[cfg(target_os = "windows")]
    {
        let mut res = winres::WindowsResource::new();

        let pkg_name = env!("CARGO_PKG_NAME");
        let pkg_version = env!("CARGO_PKG_VERSION");
        let description = env_or("AW_EXE_DESCRIPTION", "Auto change Bing wallpaper");
        let product_name = env_or("AW_EXE_PRODUCT_NAME", pkg_name);
        let author = env_or("AW_EXE_AUTHOR", "");

        let icon = icon_path();
        println!("cargo:rerun-if-env-changed=AW_ICON_PATH");
        println!("cargo:rerun-if-env-changed=AW_EXE_AUTHOR");
        println!("cargo:rerun-if-env-changed=AW_EXE_PRODUCT_NAME");
        println!("cargo:rerun-if-env-changed=AW_EXE_DESCRIPTION");
        println!("cargo:rerun-if-changed={}", icon.display());

        if icon.exists() {
            res.set_icon(icon.to_string_lossy().as_ref());
        }

        res.set("FileDescription", &description);
        res.set("ProductName", &product_name);
        res.set("ProductVersion", pkg_version);
        res.set("FileVersion", pkg_version);

        if !author.is_empty() {
            res.set("CompanyName", &author);
            res.set("LegalCopyright", &format!("© {}", author));
        }

        res.set_version_info(winres::VersionInfo::PRODUCTVERSION, parse_version_u64(pkg_version));
        res.set_version_info(winres::VersionInfo::FILEVERSION, parse_version_u64(pkg_version));

        if let Err(e) = res.compile() {
            panic!("Failed to compile Windows resources: {e}");
        }
    }
}
