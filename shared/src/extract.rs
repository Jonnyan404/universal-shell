//! 资产包解压。
//!
//! - `extract_file`：从 tar.gz / zip 挑单个成员直接落盘（raw 裸二进制直接复制）
//! - `extract_whole`：整包解压到目标目录（syncthing/frp 等含多文件场景）
//! - `list_entries`：列出包内(已解目录)的非目录条目，用于 whole 模式定位可执行文件
//!
//! 关键点：single 模式直接流式写入目标文件名，绝不先解压成包内原成员名再改名——
//! 避免包内成员名与壳自身同名时覆盖壳。

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context};
use rust_i18n::t;

/// 从 archive 提取单个文件(或裸二进制)到 dest。
pub fn extract_file(
    archive: &PathBuf,
    format: &str,
    member: Option<&str>,
    dest: &PathBuf,
) -> anyhow::Result<()> {
    match format {
        "tar.gz" | "tar" => extract_single_tar(archive, member, dest),
        "zip" => extract_single_zip(archive, member, dest),
        _ => {
            std::fs::copy(archive, dest).with_context(|| t!("err.extract.copy_bin").to_string())?;
            set_exec(dest);
            Ok(())
        }
    }
}

/// 整包解压到 dest_dir。
pub fn extract_whole(archive: &PathBuf, format: &str, dest_dir: &PathBuf) -> anyhow::Result<()> {
    std::fs::create_dir_all(dest_dir)?;
    match format {
        "tar.gz" | "tar" => extract_whole_tar(archive, dest_dir),
        "zip" => extract_whole_zip(archive, dest_dir),
        _ => anyhow::bail!(t!("err.extract.whole_unsupported", format = format)),
    }
}

/// 列出目录内所有相对非目录条目的绝对路径。
pub fn list_entries(dir: &PathBuf) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in entries.flatten() {
        let path = e.path();
        if path.is_dir() {
            out.extend(list_entries(&path));
        } else {
            out.push(path);
        }
    }
    out
}

fn extract_single_tar(archive: &PathBuf, want_member: Option<&str>, dest: &PathBuf) -> anyhow::Result<()> {
    let f = std::fs::File::open(archive).context(t!("err.extract.open_tar"))?;
    let gz = flate2::read::GzDecoder::new(f);
    let mut tar = tar::Archive::new(gz);
    let mut found: Option<tar::Entry<'_, _>> = None;
    for entry in tar.entries().context(t!("err.extract.read_tar"))? {
        let entry = entry.context(t!("err.extract.tar_entry"))?;
        let path = entry
            .path()
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        if entry.header().entry_type().is_dir() {
            continue;
        }
        let match_name = match want_member {
            Some(m) => {
                let trimmed = m.strip_prefix("./").unwrap_or(m);
                path == m || path == trimmed || path.ends_with(m)
            }
            None => true,
        };
        if match_name {
            found = Some(entry);
            break;
        }
    }
    let mut entry = found.ok_or_else(|| anyhow!(t!("err.extract.no_tar_member")))?;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut out = std::fs::File::create(dest)
        .with_context(|| t!("err.extract.create_dest", path = dest.display()).to_string())?;
    std::io::copy(&mut entry, &mut out).context(t!("err.extract.extract"))?;
    drop(out);
    set_exec(dest);
    Ok(())
}

fn extract_single_zip(archive: &PathBuf, want_member: Option<&str>, dest: &PathBuf) -> anyhow::Result<()> {
    let f = std::fs::File::open(archive).context(t!("err.extract.open_zip"))?;
    let mut z = zip::ZipArchive::new(f).context(t!("err.extract.parse_zip"))?;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let target_name = resolve_zip_name(&mut z, want_member)?;
    let mut entry = z.by_name(&target_name).context(t!("err.extract.read_zip"))?;
    let mut out = std::fs::File::create(dest).with_context(|| t!("err.extract.create_dest", path = dest.display()).to_string())?;
    std::io::copy(&mut entry, &mut out).context(t!("err.extract.extract"))?;
    drop(out);
    set_exec(dest);
    Ok(())
}

/// 在 zip 里挑选目标成员名：给定 member 按 basename 匹配，否则取第一个非目录成员
fn resolve_zip_name(z: &mut zip::ZipArchive<std::fs::File>, want_member: Option<&str>) -> anyhow::Result<String> {
    match want_member {
        Some(m) => {
            let norm = m.replace('\\', "/");
            for name in z.file_names() {
                if name == m || name == norm || name.replace('\\', "/").ends_with(&norm) {
                    return Ok(name.to_string());
                }
            }
            Err(anyhow!(t!("err.extract.no_zip_member", m = m)))
        }
        None => {
            for i in 0..z.len() {
                let f = z.by_index(i)?;
                if !f.is_dir() {
                    return Ok(f.name().to_string());
                }
            }
            Err(anyhow!(t!("err.extract.no_zip_file")))
        }
    }
}

fn extract_whole_tar(archive: &PathBuf, dest_dir: &PathBuf) -> anyhow::Result<()> {
    let f = std::fs::File::open(archive).context(t!("err.extract.open_tar"))?;
    let gz = flate2::read::GzDecoder::new(f);
    let mut tar = tar::Archive::new(gz);
    // 逐条解出，防御性去掉 ".."
    for entry in tar.entries().context(t!("err.extract.read_tar"))? {
        let mut entry = entry.context(t!("err.extract.tar_entry"))?;
        // 安全规范路径
        let rel = entry
            .path()
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let safe = safe_rel(&rel)?;
        let out_path = dest_dir.join(safe);
        if entry.header().entry_type().is_dir() {
            std::fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            entry.unpack(&out_path).context(t!("err.extract.write_entry"))?;
            set_exec(&out_path);
        }
    }
    Ok(())
}

fn extract_whole_zip(archive: &PathBuf, dest_dir: &PathBuf) -> anyhow::Result<()> {
    let f = std::fs::File::open(archive).context(t!("err.extract.open_zip"))?;
    let mut z = zip::ZipArchive::new(f).context(t!("err.extract.parse_zip"))?;
    for i in 0..z.len() {
        let mut entry = z.by_index(i)?;
        let rel = entry.name().to_string();
        let out_path = dest_dir.join(safe_rel(&rel)?);
        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(&out_path)?;
        std::io::copy(&mut entry, &mut out).context(t!("err.extract.write_entry"))?;
        set_exec(&out_path);
    }
    Ok(())
}

fn safe_rel(rel: &str) -> anyhow::Result<PathBuf> {
    let mut out = PathBuf::new();
    for comp in rel.split('/') {
        if comp == ".." || comp == "." || comp.is_empty() {
            continue;
        }
        out.push(comp);
    }
    if out.as_os_str().is_empty() {
        anyhow::bail!(t!("err.extract.illegal_path", rel = rel));
    }
    Ok(out)
}

fn set_exec(p: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755));
    }
    #[cfg(windows)]
    {
        let _ = p;
    }
}