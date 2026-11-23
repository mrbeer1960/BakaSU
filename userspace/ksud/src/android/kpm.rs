//! `ksud kpm ...`：转发给 kpatch（KernelPatch 的 CLI），由它和内核里的 kpimg 通信。

use anyhow::Result;
use std::process::Command;

const KPATCH_CANDIDATES: [&str; 3] = [
    "/data/adb/ksu/bin/kpatch",
    "/data/adb/kp-next/bin/kpatch",
    "/data/adb/modules/KPatch-Next/bin/kpatch",
];

fn kpatch_bin() -> String {
    for p in KPATCH_CANDIDATES {
        if std::path::Path::new(p).exists() {
            return p.to_string();
        }
    }
    "kpatch".to_string()
}

/// `ksud kpm <args...>` 原样转发给 `kpatch kpm <args...>`
pub fn run_main(args: Vec<String>) -> Result<()> {
    let bin = kpatch_bin();
    let mut cmd = Command::new(&bin);
    cmd.arg("kpm");
    for a in &args {
        cmd.arg(a);
    }
    let status = cmd.status()?;
    std::process::exit(status.code().unwrap_or(1));
}
