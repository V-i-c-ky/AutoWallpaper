use std::fs;
use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use crate::logger::Logger;

// ── Constants ────────────────────────────────────────────────────────────────

pub const ARCHIVE_DAYS: u32 = 10;
pub const IMAGE_QUALITY: u8 = 98;

// ── Watermark ────────────────────────────────────────────────────────────────

/// Watermark definition: either an image overlay or rendered text.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum Watermark {
    #[serde(rename = "image")]
    Image {
        #[serde(default)]
        path: String,
        #[serde(rename = "posX")]
        pos_x: f64,
        #[serde(rename = "posY")]
        pos_y: f64,
        opacity: u8,
    },
    #[serde(rename = "text")]
    Text {
        #[serde(default)]
        content: String,
        #[serde(rename = "posX")]
        pos_x: f64,
        #[serde(rename = "posY")]
        pos_y: f64,
        opacity: u8,
        font_type: String,
        font_size: u32,
        font_color: [u8; 4],
        font_weight: String,
    },
}

impl Watermark {
    pub fn default_image() -> Self {
        Self::Image {
            path: "watermark1.png".into(),
            pos_x: 2.0,
            pos_y: 1.2,
            opacity: 50,
        }
    }

    pub fn default_text() -> Self {
        Self::Text {
            content: "Sample Text Watermark".into(),
            pos_x: 2.0,
            pos_y: 1.5,
            opacity: 75,
            font_type: "arial.ttf".into(),
            font_size: 46,
            font_color: [128, 128, 128, 192],
            font_weight: "normal".into(),
        }
    }

    /// One-line summary for log output.
    pub fn summary(&self) -> String {
        match self {
            Self::Image { path, pos_x, pos_y, opacity } => {
                format!("type=image, path={path}, posX={pos_x}, posY={pos_y}, opacity={opacity}")
            }
            Self::Text { content, pos_x, pos_y, opacity, .. } => {
                format!("type=text, content={content}, posX={pos_x}, posY={pos_y}, opacity={opacity}")
            }
        }
    }
}

// ── Config ───────────────────────────────────────────────────────────────────

/// Application configuration, validated and ready to use.
#[derive(Debug, Clone, Serialize)]
pub struct Config {
    pub idx: u8,
    pub mkt: String,
    pub chk: bool,
    pub ctd: bool,
    pub wtm: bool,
    pub retry_delay: u32,
    pub retry_count: u32,
    pub watermarks: Vec<Watermark>,
    pub post_execution_apps: Vec<String>,
    pub copy_to_paths: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            idx: 0,
            mkt: "zh-CN".into(),
            chk: true,
            ctd: true,
            wtm: false,
            retry_delay: 3,
            retry_count: 10,
            watermarks: vec![Watermark::default_image(), Watermark::default_text()],
            post_execution_apps: vec![],
            copy_to_paths: vec![],
        }
    }
}

// ── Flexible JSON value parsers ──────────────────────────────────────────────

fn parse_u8(v: &Value, min: u8, max: u8, default: u8) -> u8 {
    v.as_u64()
        .map(|n| (n.min(max as u64).max(min as u64)) as u8)
        .or_else(|| {
            v.as_str()
                .and_then(|s| s.parse::<u8>().ok())
                .map(|n| n.clamp(min, max))
        })
        .unwrap_or(default)
}

fn parse_u32_min(v: &Value, min: u32, default: u32) -> u32 {
    v.as_u64()
        .map(|n| (n as u32).max(min))
        .or_else(|| {
            v.as_str()
                .and_then(|s| s.parse::<u32>().ok())
                .map(|n| n.max(min))
        })
        .unwrap_or(default)
}

fn parse_bool(v: &Value, default: bool) -> bool {
    v.as_bool().or_else(|| {
        v.as_str()
            .map(|s| matches!(s.to_ascii_lowercase().as_str(), "true" | "1" | "yes" | "on"))
    }).unwrap_or(default)
}

fn parse_watermark(v: &Value, index: usize, logger: &mut Logger) -> Option<Watermark> {
    let obj = v.as_object()?;
    let wm_type = obj.get("type")?.as_str()?;

    match wm_type {
        "image" => Some(Watermark::Image {
            path: obj.get("path").and_then(|v| v.as_str()).unwrap_or("watermark1.png").into(),
            pos_x: obj.get("posX").and_then(|v| v.as_f64()).filter(|&v| v > 0.0).unwrap_or(2.0),
            pos_y: obj.get("posY").and_then(|v| v.as_f64()).filter(|&v| v > 0.0).unwrap_or(1.2),
            opacity: obj
                .get("opacity")
                .and_then(|v| v.as_u64())
                .map(|n| n.min(100) as u8)
                .unwrap_or(50),
        }),
        "text" => {
            let font_color = obj
                .get("font_color")
                .and_then(|v| v.as_array())
                .and_then(|arr| {
                    if arr.len() == 4 {
                        let v: Vec<u8> = arr.iter().filter_map(|c| c.as_u64().map(|n| n as u8)).collect();
                        if v.len() == 4 { Some([v[0], v[1], v[2], v[3]]) } else { None }
                    } else {
                        None
                    }
                })
                .unwrap_or([128, 128, 128, 192]);

            let font_weight = obj
                .get("font_weight")
                .and_then(|v| v.as_str())
                .filter(|s| matches!(*s, "normal" | "bold" | "thin" | "light"))
                .unwrap_or("normal")
                .into();

            Some(Watermark::Text {
                content: obj.get("content").and_then(|v| v.as_str()).unwrap_or("Sample Text Watermark").into(),
                pos_x: obj.get("posX").and_then(|v| v.as_f64()).filter(|&v| v > 0.0).unwrap_or(2.0),
                pos_y: obj.get("posY").and_then(|v| v.as_f64()).filter(|&v| v > 0.0).unwrap_or(1.5),
                opacity: obj.get("opacity").and_then(|v| v.as_u64()).map(|n| n.min(100) as u8).unwrap_or(75),
                font_type: obj.get("font_type").and_then(|v| v.as_str()).unwrap_or("arial.ttf").into(),
                font_size: obj.get("font_size").and_then(|v| v.as_u64()).map(|n| (n as u32).max(1)).unwrap_or(46),
                font_color,
                font_weight,
            })
        }
        other => {
            logger.log(&format!("Watermark {}: Unknown type \"{other}\", skipping", index + 1));
            None
        }
    }
}

// ── Load / Save ──────────────────────────────────────────────────────────────

/// Load, validate, and auto-fix configuration from a JSON file.
pub fn load_config(config_path: &Path, logger: &mut Logger) -> Config {
    let default = Config::default();

    if !config_path.exists() {
        logger.log("Config file not found, creating default config");
        save_config(config_path, &default);
        return default;
    }

    let content = match fs::read_to_string(config_path) {
        Ok(c) if !c.trim().is_empty() => c,
        _ => {
            logger.log("Config file empty or unreadable, creating default");
            save_config(config_path, &default);
            return default;
        }
    };

    let value: Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            logger.log(&format!("Invalid JSON: {e}. Backing up and resetting."));
            let backup = format!("{}.bak", config_path.display());
            let _ = fs::copy(config_path, &backup);
            logger.log(&format!("Corrupted config backed up to {backup}"));
            save_config(config_path, &default);
            return default;
        }
    };

    let obj = match value.as_object() {
        Some(o) => o,
        None => {
            logger.log("Config must be a JSON object, using defaults");
            save_config(config_path, &default);
            return default;
        }
    };

    let mut fixed: Vec<String> = Vec::new();

    let idx = obj.get("idx").map(|v| {
        let val = parse_u8(v, 0, 7, default.idx);
        if v.as_u64() != Some(val as u64) { fixed.push(format!("idx (set to {val})")); }
        val
    }).unwrap_or(default.idx);

    let mkt = match obj.get("mkt").and_then(|v| v.as_str()).filter(|s| s.len() >= 2) {
        Some(s) => s.to_string(),
        None => {
            if obj.contains_key("mkt") { fixed.push(format!("mkt (reset to {})", default.mkt)); }
            default.mkt.clone()
        }
    };

    let chk = obj.get("chk").map(|v| parse_bool(v, default.chk)).unwrap_or(default.chk);
    let ctd = obj.get("ctd").map(|v| parse_bool(v, default.ctd)).unwrap_or(default.ctd);
    let wtm = obj.get("wtm").map(|v| parse_bool(v, default.wtm)).unwrap_or(default.wtm);

    let retry_delay = obj.get("retry_delay").map(|v| {
        let val = parse_u32_min(v, 1, default.retry_delay);
        if v.as_u64().is_none_or(|n| n as u32 != val) { fixed.push(format!("retry_delay (set to {val})")); }
        val
    }).unwrap_or(default.retry_delay);
    let retry_count = obj.get("retry_count").map(|v| {
        let val = parse_u32_min(v, 1, default.retry_count);
        if v.as_u64().is_none_or(|n| n as u32 != val) { fixed.push(format!("retry_count (set to {val})")); }
        val
    }).unwrap_or(default.retry_count);

    let watermarks = if let Some(arr) = obj.get("watermarks").and_then(|v| v.as_array()) {
        arr.iter()
            .enumerate()
            .filter_map(|(i, v)| parse_watermark(v, i, logger))
            .collect()
    } else if obj.contains_key("watermarks") {
        fixed.push("watermarks (invalid format, reset to empty)".into());
        vec![]
    } else {
        default.watermarks.clone()
    };

    let post_execution_apps = obj
        .get("post_execution_apps")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();

    let copy_to_paths = obj
        .get("copy_to_paths")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();

    if !fixed.is_empty() {
        logger.log(&format!("Fixed config values: {}", fixed.join(", ")));
    }

    let config = Config {
        idx, mkt, chk, ctd, wtm, retry_delay, retry_count,
        watermarks, post_execution_apps, copy_to_paths,
    };

    // Detect and fill missing keys
    let default_json = match serde_json::to_value(&default) {
        Ok(v) => v,
        Err(_) => return config,
    };
    let mut needs_update = false;
    if let Some(default_obj) = default_json.as_object() {
        for key in default_obj.keys() {
            if !obj.contains_key(key) {
                logger.log(&format!("Missing config key \"{key}\", added with default value"));
                needs_update = true;
            }
        }
    }
    if needs_update {
        save_config(config_path, &config);
        logger.log("Config file updated with missing keys");
    }

    config
}

fn save_config(path: &Path, config: &Config) {
    if let Ok(json) = serde_json::to_string_pretty(config) {
        let _ = fs::write(path, json);
    }
}
