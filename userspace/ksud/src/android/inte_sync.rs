//! `ksud inte-sync`：同步 OPPO inte.ko 的“合法指纹表”。
//!
//! 行为与 inte.ko_sync（kows）一致：不修改任何 .ko 文件，只把
//! `kh_value_sys.bin` / `kh_value_vendor.bin` 中记录的 sha256 更新为
//! 当前文件的 sha256（FIX）；表里没有的模块追加一条（ADD）。
//! 首次运行会把原表备份到 `/data/backup/<name>.initial.bak`，最后重载 inte 模块。
//!
//! 为降低开机开销，用 (ctime, size) 缓存上一次的 sha256：
//! 文件没有被改动就不重算；改过（含 touch 改 mtime 后再改回来）会导致 ctime 变化，
//! 从而自动失效重算。

use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use log::{info, warn};

const SYS_TABLE: &str = "/data/oplus/persist_hash/kh_value_sys.bin";
const VND_TABLE: &str = "/data/vendor/persist_hash/kh_value_vendor.bin";
const CACHE_FILE: &str = "/data/adb/ksu/inte_sync.cache";
const BACKUP_DIR: &str = "/data/backup";
const INTE_KO_CANDIDATES: [&str; 2] = [
    "/vendor_dlkm/lib/modules/inte.ko",
    "/vendor/lib/modules/inte.ko",
];
/// sys 表：只涉及系统 GKI 模块
const SYS_DIRS: [&str; 2] = ["/system_dlkm/lib/modules", "/system/lib/modules"];
/// vendor 表：只涉及厂商分区模块（不含 /data 下的第三方模块）
const VND_DIRS: [&str; 8] = [
    "/vendor_dlkm/lib/modules",
    "/vendor/lib/modules",
    "/odm/lib/modules",
    "/product/lib/modules",
    "/my_product/lib/modules",
    "/my_stock/lib/modules",
    "/my_heytap/lib/modules",
    "/system_ext/lib/modules",
];
const NAME_LEN: usize = 40;
const HASH_LEN: usize = 32;
const ENTRY_LEN: usize = NAME_LEN + HASH_LEN;

#[derive(Clone)]
struct Entry {
    name: String,
    hash: String,
}

#[derive(Clone)]
struct Cached {
    ctime: i64,
    ctime_nsec: i64,
    size: u64,
    hash: String,
}

fn unhex(s: &str) -> Option<[u8; HASH_LEN]> {
    if s.len() != HASH_LEN * 2 {
        return None;
    }
    let mut out = [0u8; HASH_LEN];
    let b = s.as_bytes();
    for i in 0..HASH_LEN {
        let hi = (b[i * 2] as char).to_digit(16)?;
        let lo = (b[i * 2 + 1] as char).to_digit(16)?;
        out[i] = ((hi << 4) | lo) as u8;
    }
    Some(out)
}

fn to_hex(h: &[u8]) -> String {
    h.iter().map(|b| format!("{b:02x}")).collect()
}

fn read_table(path: &str) -> Result<Vec<Entry>> {
    let data = fs::read(path).with_context(|| format!("read {path}"))?;
    if data.len() < 4 {
        anyhow::bail!("{path}: too short");
    }
    let count = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
    if data.len() < 4 + count * ENTRY_LEN {
        anyhow::bail!("{path}: truncated ({count} entries)");
    }
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let off = 4 + i * ENTRY_LEN;
        let raw_name = &data[off..off + NAME_LEN];
        let end = raw_name.iter().position(|&c| c == 0).unwrap_or(NAME_LEN);
        let name = String::from_utf8_lossy(&raw_name[..end]).to_string();
        let hash = to_hex(&data[off + NAME_LEN..off + ENTRY_LEN]);
        out.push(Entry { name, hash });
    }
    Ok(out)
}

fn write_table(path: &str, entries: &[Entry]) -> Result<()> {
    let mut buf = Vec::with_capacity(4 + entries.len() * ENTRY_LEN);
    buf.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for e in entries {
        let mut name = [0u8; NAME_LEN];
        let nb = e.name.as_bytes();
        let n = nb.len().min(NAME_LEN);
        name[..n].copy_from_slice(&nb[..n]);
        buf.extend_from_slice(&name);
        let hash = unhex(&e.hash).unwrap_or([0u8; HASH_LEN]);
        buf.extend_from_slice(&hash);
    }

    // 目标文件的属主/权限必须保持（inte 以 system 身份读取）
    let (uid, gid) = match fs::metadata(path) {
        Ok(m) => (m.uid(), m.gid()),
        Err(_) => (1000, 0),
    };
    let tmp = format!("{path}.ksud-tmp");
    fs::write(&tmp, &buf).with_context(|| format!("write {tmp}"))?;
    let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
    if let Ok(c_tmp) = std::ffi::CString::new(tmp.as_str()) {
        unsafe {
            libc::chown(c_tmp.as_ptr(), uid, gid);
        }
    }
    fs::rename(&tmp, path).with_context(|| format!("rename to {path}"))?;
    Ok(())
}

fn build_id_of_module(name: &str) -> Option<Vec<u8>> {
    let p = format!("/sys/module/{name}/notes/.note.gnu.build-id");
    let d = fs::read(p).ok()?;
    if d.len() < 12 {
        return None;
    }
    let namesz = u32::from_le_bytes([d[0], d[1], d[2], d[3]]) as usize;
    let descsz = u32::from_le_bytes([d[4], d[5], d[6], d[7]]) as usize;
    let mut off = 12 + namesz;
    off = (off + 3) & !3;
    if descsz == 0 || off + descsz > d.len() {
        return None;
    }
    Some(d[off..off + descsz].to_vec())
}

fn file_contains(path: &Path, needle: &[u8]) -> bool {
    let Ok(data) = fs::read(path) else {
        return false;
    };
    if needle.is_empty() || data.len() < needle.len() {
        return false;
    }
    data.windows(needle.len()).any(|w| w == needle)
}

/// 磁盘文件与内核里已加载模块是否同版本（比对 build-id，返回 Some(内存id, 说明) 表示不一致）
fn stale_check(name: &str, path: &Path) -> Option<(String, String)> {
    let module = name.strip_suffix(".ko").unwrap_or(name);
    let id = build_id_of_module(module)?;
    let hexid = to_hex(&id);
    if file_contains(path, &id) {
        None
    } else {
        Some((hexid, "磁盘文件里找不到该 build-id".to_string()))
    }
}

fn sha256_of(path: &Path) -> Result<String> {
    let data = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    Ok(sha256::digest(&data))
}

fn meta_of(path: &Path) -> Result<(i64, i64, u64)> {
    let m = fs::metadata(path)?;
    Ok((m.ctime(), m.ctime_nsec(), m.len()))
}

fn load_cache() -> HashMap<String, Cached> {
    let mut map = HashMap::new();
    let Ok(data) = fs::read_to_string(CACHE_FILE) else {
        return map;
    };
    for line in data.lines() {
        let mut it = line.split('\t');
        let (Some(p), Some(c), Some(ns), Some(sz), Some(h)) =
            (it.next(), it.next(), it.next(), it.next(), it.next())
        else {
            continue;
        };
        let (Ok(ctime), Ok(ctime_nsec), Ok(size)) = (c.parse(), ns.parse(), sz.parse()) else {
            continue;
        };
        map.insert(
            p.to_string(),
            Cached {
                ctime,
                ctime_nsec,
                size,
                hash: h.to_string(),
            },
        );
    }
    map
}

fn save_cache(map: &HashMap<String, Cached>) {
    let mut out = String::new();
    for (p, c) in map {
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\n",
            p, c.ctime, c.ctime_nsec, c.size, c.hash
        ));
    }
    if let Err(e) = fs::write(CACHE_FILE, out) {
        warn!("write cache failed: {e}");
    }
}

fn find_module(name: &str, dirs: &[&str]) -> Option<PathBuf> {
    for dir in dirs {
        let p = Path::new(dir).join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn loaded_modules() -> Vec<String> {
    let Ok(data) = fs::read_to_string("/proc/modules") else {
        return Vec::new();
    };
    data.lines()
        .filter_map(|l| l.split_whitespace().next().map(|s| s.to_string()))
        .collect()
}

fn reload_inte() {
    let ko = INTE_KO_CANDIDATES
        .iter()
        .find(|p| Path::new(p).is_file())
        .copied();
    let _ = Command::new("rmmod").arg("inte").status();
    if let Some(ko) = ko {
        let _ = Command::new("insmod").arg(ko).status();
    }
}

fn backup_once(table: &str) -> Result<()> {
    let name = Path::new(table)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let bak = format!("{BACKUP_DIR}/{name}.initial.bak");
    if Path::new(&bak).exists() {
        return Ok(());
    }
    if Path::new(table).is_file() {
        fs::copy(table, &bak).with_context(|| format!("backup {table}"))?;
        let _ = fs::set_permissions(&bak, fs::Permissions::from_mode(0o600));
        info!("inte-sync: backed up {table} -> {bak}");
    }
    Ok(())
}

fn sync_one(
    table: &str,
    dirs: &[&str],
    check: bool,
    deep: bool,
    cache: &mut HashMap<String, Cached>,
) -> Result<(usize, usize, usize, usize, usize)> {
    let mut entries = read_table(table)?;
    backup_once(table)?;

    let (mut ok, mut fix, mut add, mut miss, mut stale) = (0, 0, 0, 0, 0);
    let mut dirty = false;

    for e in entries.iter_mut() {
        let Some(path) = find_module(&e.name, dirs) else {
            miss += 1;
            continue;
        };
        let key = path.to_string_lossy().to_string();
        let (ctime, ctime_nsec, size) = meta_of(&path)?;
        let hash = match cache.get(&key) {
            Some(c) if c.ctime == ctime && c.ctime_nsec == ctime_nsec && c.size == size => {
                c.hash.clone()
            }
            _ => {
                let h = sha256_of(&path)?;
                cache.insert(
                    key,
                    Cached {
                        ctime,
                        ctime_nsec,
                        size,
                        hash: h.clone(),
                    },
                );
                h
            }
        };
        let changed = hash != e.hash;
        if changed {
            fix += 1;
            info!(
                "inte-sync: {} {} {} -> {}",
                if check { "WOULD-FIX" } else { "FIX" },
                e.name,
                e.hash,
                hash
            );
            if !check {
                e.hash = hash;
                dirty = true;
            }
        } else {
            ok += 1;
        }
        // 磁盘文件与内核里已加载模块是否同版本（原版也会比对 build-id / srcversion）
        if changed || deep {
            if let Some((mem, disk)) = stale_check(&e.name, &path) {
                stale += 1;
                warn!(
                    "inte-sync: STALE {} 内存={} 磁盘={}（磁盘变了但模块未重载）",
                    e.name, mem, disk
                );
            }
        }
    }

    // 表里没有、但已经加载的模块 → ADD
    for name in loaded_modules() {
        let fname = format!("{name}.ko");
        if entries.iter().any(|e| e.name == fname || e.name == name) {
            continue;
        }
        let Some(path) = find_module(&fname, dirs) else {
            continue;
        };
        let Ok(h) = sha256_of(&path) else {
            continue;
        };
        add += 1;
        info!(
            "inte-sync: {} {} {}",
            if check { "WOULD-ADD" } else { "ADD" },
            fname,
            h
        );
        if !check {
            entries.push(Entry {
                name: fname,
                hash: h,
            });
            dirty = true;
        }
    }

    if dirty && !check {
        write_table(table, &entries)?;
    }
    Ok((ok, fix, add, miss, stale))
}

/// `ksud inte-sync [--check]`
pub fn run_main(check: bool, deep: bool) -> Result<()> {
    let mut cache = load_cache();
    let mut total = (0usize, 0usize, 0usize, 0usize, 0usize);
    for (table, dirs) in [(SYS_TABLE, &SYS_DIRS[..]), (VND_TABLE, &VND_DIRS[..])] {
        if !Path::new(table).is_file() {
            warn!("inte-sync: {table} not found, skip");
            continue;
        }
        match sync_one(table, dirs, check, deep, &mut cache) {
            Ok((ok, fix, add, miss, stale)) => {
                info!("inte-sync {table}: ok={ok} fix={fix} add={add} miss={miss}");
                total.0 += ok;
                total.1 += fix;
                total.2 += add;
                total.3 += miss;
                total.4 += stale;
            }
            Err(e) => warn!("inte-sync {table} failed: {e:#}"),
        }
    }
    save_cache(&cache);
    println!(
        "inte-sync: ok={} fix={} add={} miss={} stale={}{}",
        total.0,
        total.1,
        total.2,
        total.3,
        total.4,
        if check { " (check only)" } else { "" }
    );
    if !check && (total.1 > 0 || total.2 > 0) {
        reload_inte();
    }
    Ok(())
}

/// post-fs-data 阶段自动执行一次（对应 inte.ko_sync 的 post-fs-data.sh）。
pub fn run_post_fs_data() {
    if !Path::new(SYS_TABLE).is_file() && !Path::new(VND_TABLE).is_file() {
        return;
    }
    if let Err(e) = run_main(false, false) {
        warn!("inte-sync post-fs-data failed: {e:#}");
    }
}
