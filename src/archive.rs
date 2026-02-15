use std::fs;
use std::path::Path;

use chrono::Local;

use crate::logger::Logger;

/// Move date-named folders older than `days` into a yearly archive structure.
pub fn archive_old_folders(
    base_folder: &Path,
    archive_folder: &Path,
    logger: &mut Logger,
    days: u32,
) {
    let _ = fs::create_dir_all(archive_folder);

    let cutoff = Local::now().date_naive() - chrono::Duration::days(days as i64);
    let mut count = 0u32;

    let entries = match fs::read_dir(base_folder) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        if !entry.file_type().is_ok_and(|ft| ft.is_dir()) {
            continue;
        }

        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Only process folders matching the date pattern YYYY.MM.DD
        if let Ok(date) = chrono::NaiveDate::parse_from_str(&name_str, "%Y.%m.%d") {
            if date < cutoff {
                let year_folder = archive_folder.join(date.format("%Y").to_string());
                let _ = fs::create_dir_all(&year_folder);

                if fs::rename(entry.path(), year_folder.join(&*name_str)).is_ok() {
                    count += 1;
                }
            }
        }
    }

    logger.log(&format!("Archived {count} folders"));
}
