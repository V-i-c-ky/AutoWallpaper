#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod archive;
mod config;
mod download;
mod logger;
mod wallpaper;
mod watermark;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::Local;
use serde::{Deserialize, Serialize};

use archive::archive_old_folders;
use config::{load_config, ARCHIVE_DAYS};
use download::download_file;
use logger::Logger;
use wallpaper::{get_current_wallpaper, set_wallpaper};
use watermark::add_watermarks;

const BING_API: &str = "https://www.bing.com/HPImageArchive.aspx?n=1";

// ── Status tracking ──────────────────────────────────────────────────────────

#[derive(Debug, Default, Serialize, Deserialize)]
struct Status {
    #[serde(default)]
    completed: bool,
    #[serde(default)]
    downloaded: bool,
    #[serde(default)]
    watermark_added: bool,
    #[serde(default)]
    wallpaper_set: bool,
    #[serde(default)]
    completed_time: Option<String>,
    #[serde(default)]
    download_time: Option<String>,
}

fn load_status(path: &Path) -> Status {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_status(path: &Path, status: &Status) {
    if let Ok(json) = serde_json::to_string_pretty(status) {
        let _ = fs::write(path, json);
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn get_base_path() -> PathBuf {
    env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| env::current_dir().unwrap_or_default())
}

/// Verify that an image file exists, is large enough, and can be decoded.
fn verify_image(path: &Path, logger: &mut Logger) -> bool {
    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return false,
    };
    if meta.len() < 10 * 1024 {
        logger.log(&format!(
            "Image file too small ({} bytes): {}",
            meta.len(),
            path.display()
        ));
        return false;
    }
    match image::open(path) {
        Ok(_) => true,
        Err(e) => {
            logger.log(&format!("Image verification failed: {e}"));
            false
        }
    }
}

/// Check whether today's wallpaper has already been successfully applied.
fn check_already_completed(dfolder: &Path, name: &str, logger: &mut Logger) -> bool {
    let image_path = dfolder.join(format!("{name}.jpg"));
    let status_file = dfolder.join("status.json");

    let mut status = load_status(&status_file);

    if !status.completed {
        return false;
    }
    if !verify_image(&image_path, logger) {
        logger.log("Previous image file is missing or corrupted, will re-download");
        return false;
    }

    if let Some(current) = get_current_wallpaper() {
        let current_norm = normalize_path(&current);
        let abs = fs::canonicalize(&image_path).unwrap_or_else(|_| image_path.clone());
        let abs_str = abs.to_string_lossy();
        let clean = abs_str.strip_prefix(r"\\?\").unwrap_or(&abs_str);
        let target_norm = normalize_path(clean);

        if current_norm != target_norm {
            logger.log("Current wallpaper differs from today's image, will re-apply");
            status.wallpaper_set = false;
            save_status(&status_file, &status);
            return false;
        }
    }

    logger.log("Today's wallpaper already completed and verified");
    true
}

fn normalize_path(path: &str) -> String {
    wallpaper::normalize_path(path)
}

fn copy_to_desktop(image_path: &Path, logger: &mut Logger) {
    if let Ok(home) = env::var("USERPROFILE") {
        let dest = PathBuf::from(home).join("Desktop").join("wallpaper.jpg");
        match fs::copy(image_path, &dest) {
            Ok(_) => logger.log("Wallpaper copied to desktop"),
            Err(e) => logger.log(&format!("Failed to copy wallpaper to desktop: {e}")),
        }
    }
}

/// Expand `%VAR%` style environment variables in a string.
/// `%%` is collapsed to a literal `%`.
fn expand_env(s: &str) -> String {
    let mut result = s.to_string();
    let mut idx = 0;
    while let Some(start) = result[idx..].find('%') {
        let start = start + idx;
        if let Some(end) = result[start + 1..].find('%') {
            let var = &result[start + 1..start + 1 + end];
            if var.is_empty() {
                // %% → %
                result = format!("{}%{}", &result[..start], &result[start + 2..]);
                idx = start + 1;
            } else {
                let val = env::var(var).unwrap_or_default();
                result = format!("{}{val}{}", &result[..start], &result[start + 2 + end..]);
                idx = start + val.len();
            }
        } else {
            break;
        }
    }
    result
}

fn run_post_execution_apps(apps: &[String], logger: &mut Logger) {
    for app in apps {
        let expanded = expand_env(app);
        logger.log(&format!("Trying to execute {expanded}"));
        let mut command = if cfg!(target_os = "windows") {
            let mut c = Command::new("cmd");
            c.arg("/C").arg(&expanded);
            c
        } else {
            let mut c = Command::new("sh");
            c.arg("-c").arg(&expanded);
            c
        };

        match command.spawn().and_then(|mut c| c.wait()) {
            Ok(s) => logger.log(&format!(
                "Executed {expanded} with code {}",
                s.code().unwrap_or(-1)
            )),
            Err(e) => logger.log(&format!("Failed to execute {expanded}: {e}")),
        }
    }
}

// ── Main logic ───────────────────────────────────────────────────────────────

fn run(logger: &mut Logger) {
    let name = Local::now().format("%Y.%m.%d").to_string();
    let appdata = env::var("APPDATA").unwrap_or_default();
    let folder = PathBuf::from(&appdata).join("AutoWallpaper");
    let dfolder = folder.join(&name);
    let archive_path = folder.join("Archive");
    let _ = fs::create_dir_all(&dfolder);

    let status_file = dfolder.join("status.json");
    let image_path = dfolder.join(format!("{name}.jpg"));

    // Archive old folders
    archive_old_folders(&folder, &archive_path, logger, ARCHIVE_DAYS);

    // Load config
    let base_path = get_base_path();
    let config = load_config(&base_path.join("config.json"), logger);

    // Log config summary
    let wm_details = if config.watermarks.is_empty() {
        "No watermarks configured".into()
    } else {
        config
            .watermarks
            .iter()
            .enumerate()
            .map(|(i, wm)| format!("Watermark {}: {}", i + 1, wm.summary()))
            .collect::<Vec<_>>()
            .join(", ")
    };
    logger.log(&format!(
        "Config: idx={}, mkt={}, chk={}, ctd={}, wtm={}, retry_delay={}, retry_count={}, {wm_details}, post_execution_apps={:?}, copy_to_paths={:?}",
        config.idx, config.mkt, config.chk, config.ctd, config.wtm,
        config.retry_delay, config.retry_count,
        config.post_execution_apps, config.copy_to_paths,
    ));

    // Skip if already completed
    if config.chk && check_already_completed(&dfolder, &name, logger) {
        return;
    }

    let mut status = load_status(&status_file);

    // Download if needed
    if !verify_image(&image_path, logger) {
        let api_url = format!("{BING_API}&mkt={}&idx={}&format=js", config.mkt, config.idx);
        let api_json = dfolder.join("api.json");

        if !download_file(&api_url, &api_json, logger, config.retry_delay, config.retry_count) {
            logger.log("Failed to download API files");
            return;
        }

        let link = fs::read_to_string(&api_json)
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| v["images"][0]["urlbase"].as_str().map(String::from));

        let link = match link {
            Some(l) => l,
            None => {
                logger.log("Failed to parse download link from API response");
                return;
            }
        };

        let full_url = format!("https://www.bing.com{link}_UHD.jpg");
        if !download_file(&full_url, &image_path, logger, config.retry_delay, config.retry_count) {
            logger.log("Failed to download image");
            return;
        }

        if !verify_image(&image_path, logger) {
            logger.log("Downloaded image is corrupted, aborting");
            let _ = fs::remove_file(&image_path);
            return;
        }

        status.downloaded = true;
        status.download_time = Some(Local::now().to_rfc3339());
        save_status(&status_file, &status);
        logger.log("Image downloaded and verified");
    } else {
        logger.log("Using existing valid image file");
    }

    // Watermarks
    if config.wtm && !status.watermark_added {
        let original = dfolder.join(format!("{name}_original.jpg"));
        if !original.exists() {
            match fs::copy(&image_path, &original) {
                Ok(_) => logger.log(&format!("Original image saved as {}", original.display())),
                Err(e) => logger.log(&format!("Failed to save original: {e}")),
            }
        }
        add_watermarks(&image_path, &config.watermarks, &base_path, logger);
        status.watermark_added = true;
        save_status(&status_file, &status);
    }

    // Copy to configured paths
    for path in &config.copy_to_paths {
        let expanded = expand_env(path);
        let ep = Path::new(&expanded);
        let target = if ep.extension().is_some() {
            PathBuf::from(&expanded)
        } else {
            let _ = fs::create_dir_all(&expanded);
            PathBuf::from(&expanded).join(format!("{name}.jpg"))
        };
        match fs::copy(&image_path, &target) {
            Ok(_) => logger.log(&format!("Image copied to {}", target.display())),
            Err(e) => logger.log(&format!("Failed to copy image to {expanded}: {e}")),
        }
    }

    // Set wallpaper
    let wallpaper_ok = set_wallpaper(&image_path, logger);
    status.wallpaper_set = wallpaper_ok;

    if !wallpaper_ok {
        logger.log("Warning: Wallpaper setting may have failed, will retry next run");
    }

    // Copy to desktop
    if config.ctd {
        copy_to_desktop(&image_path, logger);
    }

    // Post-execution apps
    run_post_execution_apps(&config.post_execution_apps, logger);

    // Mark completed
    if wallpaper_ok {
        status.completed = true;
        status.completed_time = Some(Local::now().to_rfc3339());
    }
    save_status(&status_file, &status);

    if wallpaper_ok {
        logger.log("All tasks completed");
    }
}

fn main() {
    let name = Local::now().format("%Y.%m.%d").to_string();
    let appdata = env::var("APPDATA").unwrap_or_default();
    let dfolder = PathBuf::from(&appdata).join("AutoWallpaper").join(&name);
    let _ = fs::create_dir_all(&dfolder);
    let log_path = dfolder.join(format!("{name}.log"));

    let mut logger = Logger::new(&log_path);
    logger.log("********************Log Start********************");

    run(&mut logger);

    logger.log("*********************Log End*********************");
}
