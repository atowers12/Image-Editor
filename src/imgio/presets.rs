//! Named edit presets ("looks"): an `EditParams` saved as JSON under
//! %APPDATA%\photo-editor\presets\<name>.json. Applying a preset copies its
//! visual edits onto the current photo (geometry and rating are not part of
//! a preset).

use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::engine::params::EditParams;

fn dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("photo-editor").join("presets"))
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim()
        .to_string()
}

/// A preset stores only visual edits: strip geometry, crop, and metadata so
/// a look applies cleanly to any photo regardless of orientation.
pub fn strip_for_preset(p: &EditParams) -> EditParams {
    let mut e = p.clone();
    e.reset_geometry();
    e.rating = 0;
    e.flag = crate::engine::params::Flag::None;
    e
}

/// List available preset names (file stems), sorted.
pub fn list() -> Vec<String> {
    let Some(d) = dir() else {
        return Vec::new();
    };
    let mut names: Vec<String> = std::fs::read_dir(d)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().map(|x| x == "json").unwrap_or(false))
                .filter_map(|e| {
                    e.path()
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                })
                .collect()
        })
        .unwrap_or_default();
    names.sort_by_key(|n| n.to_lowercase());
    names
}

pub fn save(name: &str, params: &EditParams) -> Result<()> {
    let name = sanitize(name);
    if name.is_empty() {
        anyhow::bail!("preset name is empty");
    }
    let Some(d) = dir() else {
        anyhow::bail!("APPDATA not set; cannot save presets");
    };
    std::fs::create_dir_all(&d)?;
    let json = serde_json::to_string_pretty(&strip_for_preset(params))?;
    std::fs::write(d.join(format!("{name}.json")), json)
        .with_context(|| format!("writing preset {name}"))?;
    Ok(())
}

pub fn load(name: &str) -> Result<EditParams> {
    let Some(d) = dir() else {
        anyhow::bail!("APPDATA not set");
    };
    let text = std::fs::read_to_string(d.join(format!("{}.json", sanitize(name))))
        .with_context(|| format!("reading preset {name}"))?;
    Ok(serde_json::from_str(&text)?)
}

pub fn delete(name: &str) -> Result<()> {
    if let Some(d) = dir() {
        let path = d.join(format!("{}.json", sanitize(name)));
        if path.exists() {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_removes_geometry_keeps_edits() {
        let mut p = EditParams::default();
        p.exposure = 1.0;
        p.rotate90 = 2;
        p.crop = [0.1, 0.1, 0.5, 0.5];
        p.rating = 4;
        let s = strip_for_preset(&p);
        assert_eq!(s.exposure, 1.0);
        assert_eq!(s.rotate90, 0);
        assert_eq!(s.crop, crate::engine::params::CROP_FULL);
        assert_eq!(s.rating, 0);
    }

    #[test]
    fn sanitize_strips_path_chars() {
        assert_eq!(sanitize("My/Look:1"), "My_Look_1");
    }
}
