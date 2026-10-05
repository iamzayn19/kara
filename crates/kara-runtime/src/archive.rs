//! Archive extraction that refuses entries escaping the destination
//! (absolute paths, `..`, or symlinks pointing outside).

use std::path::{Component, Path, PathBuf};

fn safe_join(dest: &Path, entry: &Path) -> anyhow::Result<PathBuf> {
    let mut out = dest.to_path_buf();
    for c in entry.components() {
        match c {
            Component::Normal(p) => out.push(p),
            Component::CurDir => {}
            _ => anyhow::bail!("archive entry {} escapes the destination", entry.display()),
        }
    }
    Ok(out)
}

pub fn extract(archive: &Path, dest: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dest)?;
    let name = archive.to_string_lossy().to_ascii_lowercase();
    if name.ends_with(".zip") {
        extract_zip(archive, dest)
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        extract_tar_gz(archive, dest)
    } else {
        anyhow::bail!("unsupported archive format: {}", archive.display())
    }
}

fn extract_tar_gz(archive: &Path, dest: &Path) -> anyhow::Result<()> {
    let f = std::fs::File::open(archive)?;
    let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(f));
    ar.set_preserve_permissions(true);
    for entry in ar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let target = safe_join(dest, &path)?;
        let kind = entry.header().entry_type();
        if kind.is_symlink() || kind.is_hard_link() {
            let link = entry
                .link_name()?
                .ok_or_else(|| anyhow::anyhow!("link without target"))?
                .into_owned();
            // Resolve relative to the link's directory and require it to stay inside.
            let base = path.parent().unwrap_or(Path::new(""));
            let resolved = if link.is_absolute() {
                anyhow::bail!(
                    "archive link {} -> {} is absolute",
                    path.display(),
                    link.display()
                );
            } else {
                base.join(&link)
            };
            let mut depth: i32 = 0;
            for c in resolved.components() {
                match c {
                    Component::ParentDir => depth -= 1,
                    Component::Normal(_) => depth += 1,
                    _ => {}
                }
                if depth < 0 {
                    anyhow::bail!("archive link {} escapes the destination", path.display());
                }
            }
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        entry.unpack(&target)?;
    }
    Ok(())
}

fn extract_zip(archive: &Path, dest: &Path) -> anyhow::Result<()> {
    let f = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(f)?;
    for i in 0..zip.len() {
        let mut file = zip.by_index(i)?;
        let Some(rel) = file.enclosed_name() else {
            anyhow::bail!("zip entry {} escapes the destination", file.name());
        };
        let target = safe_join(dest, &rel)?;
        if file.is_dir() {
            std::fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(&target)?;
        std::io::copy(&mut file, &mut out)?;
        #[cfg(unix)]
        if let Some(mode) = file.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(mode & 0o755))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tar_gz(entries: &[(&str, &[u8])]) -> tempfile::NamedTempFile {
        let f = tempfile::Builder::new()
            .suffix(".tar.gz")
            .tempfile()
            .unwrap();
        let enc = flate2::write::GzEncoder::new(f.reopen().unwrap(), flate2::Compression::fast());
        let mut b = tar::Builder::new(enc);
        for (name, data) in entries {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o755);
            // Write the raw name to allow hostile paths in tests.
            let bytes = name.as_bytes();
            h.as_old_mut().name[..bytes.len()].copy_from_slice(bytes);
            h.set_cksum();
            b.append(&h, *data).unwrap();
        }
        b.into_inner().unwrap().finish().unwrap();
        f
    }

    #[test]
    fn extracts_normal_archive() {
        let ar = tar_gz(&[
            ("llama-b1/llama-server", b"bin"),
            ("llama-b1/lib.dylib", b"lib"),
        ]);
        let dest = tempfile::tempdir().unwrap();
        extract(ar.path(), dest.path()).unwrap();
        assert_eq!(
            std::fs::read(dest.path().join("llama-b1/llama-server")).unwrap(),
            b"bin"
        );
    }

    #[test]
    fn rejects_traversal() {
        let ar = tar_gz(&[("../../evil.sh", b"x")]);
        let dest = tempfile::tempdir().unwrap();
        assert!(extract(ar.path(), dest.path()).is_err());
        assert!(!dest.path().parent().unwrap().join("evil.sh").exists());
    }

    #[test]
    fn zip_roundtrip_and_traversal() {
        let f = tempfile::Builder::new().suffix(".zip").tempfile().unwrap();
        {
            let mut z = zip::ZipWriter::new(f.reopen().unwrap());
            let opts = zip::write::SimpleFileOptions::default();
            z.start_file("bin/llama-server.exe", opts).unwrap();
            z.write_all(b"exe").unwrap();
            z.finish().unwrap();
        }
        let dest = tempfile::tempdir().unwrap();
        extract(f.path(), dest.path()).unwrap();
        assert_eq!(
            std::fs::read(dest.path().join("bin/llama-server.exe")).unwrap(),
            b"exe"
        );

        let g = tempfile::Builder::new().suffix(".zip").tempfile().unwrap();
        {
            let mut z = zip::ZipWriter::new(g.reopen().unwrap());
            z.start_file("../escape.txt", zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(b"x").unwrap();
            z.finish().unwrap();
        }
        assert!(extract(g.path(), dest.path()).is_err());
    }
}
