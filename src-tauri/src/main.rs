#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::Path;

/// 忘记主密码时的强制重置。刻意不做成 UI 按钮：
/// 主密码就是整库的加密密钥，重置只能是"丢掉旧库从头来"，
/// 所以要求必须先 cd 到数据目录、再手敲命令，让这件事做不成一次误点。
fn reset_in_cwd() -> i32 {
    let db = Path::new(switchboard_lib::DB_FILE);
    if !db.exists() {
        eprintln!(
            "当前目录下没有 {}。\n请先 cd 到 Switchboard 的数据目录（App 登录页的\"忘记主密码\"里有完整路径）再执行。",
            switchboard_lib::DB_FILE
        );
        return 1;
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let backup = format!("{}.locked-{}", switchboard_lib::DB_FILE, stamp);
    if let Err(e) = std::fs::rename(db, &backup) {
        eprintln!("重置失败：{e}");
        return 1;
    }
    println!(
        "已重置。\n  旧账号库改名为：{backup}\n  它仍然用旧主密码加密，想不起来密码就解不开，确认不要了再删。\n\n现在重新打开 Switchboard，会回到\"创建主账号\"。\n注意：各账号留在本机的登录数据没有动。如需彻底清理，手动删掉 ~/Library/WebKit/com.switchboard.app/WebsiteDataStore（macOS）或数据目录下的 profiles/（Windows / Linux）。"
    );
    0
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("reset") {
        std::process::exit(reset_in_cwd());
    }
    switchboard_lib::run()
}
