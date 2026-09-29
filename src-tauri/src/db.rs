use crate::models::{Account, PlatformConfig};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

/// 打开（必要时创建）加密库。主密码直接作为 SQLCipher 的 key——
/// SQLCipher 内部已经是 PBKDF2-HMAC-SHA512 256k 轮派生，不再另外叠一层 KDF。
/// 密码不对时 `PRAGMA key` 本身不报错，第一次真正读页面才会失败，所以下面用
/// 一条 sqlite_master 查询做校验，这也就是 Unlock 屏的密码验证逻辑。
pub fn open(path: &Path, password: &str) -> Result<Connection, String> {
    let conn = open_for_unlock(path, password)?;
    prepare(&conn)?;
    Ok(conn)
}

/// 验证密码并读取加密库，但不做任何建表或默认配置写入。
/// 解锁流程先用它读取区域设置，通过网络检查后再调用 `prepare`。
pub fn open_for_unlock(path: &Path, password: &str) -> Result<Connection, String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let conn = Connection::open(path).map_err(|e| e.to_string())?;
    conn.pragma_update(None, "key", password)
        .map_err(|e| e.to_string())?;
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
        r.get::<_, i64>(0)
    })
    .map_err(|_| "主密码不正确".to_string())?;
    Ok(conn)
}

pub fn prepare(conn: &Connection) -> Result<(), String> {
    init(conn)?;
    seed_platform_configs(conn)
}

/// 修改主密码：SQLCipher 用新 key 重新加密整库。
pub fn rekey(conn: &Connection, new_password: &str) -> Result<(), String> {
    conn.pragma_update(None, "rekey", new_password)
        .map_err(|e| e.to_string())
}

fn init(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS meta (
            key   TEXT PRIMARY KEY,
            value TEXT
         );
         CREATE TABLE IF NOT EXISTS accounts (
            id             TEXT PRIMARY KEY,
            platform       TEXT NOT NULL,
            owner_type     TEXT NOT NULL,
            related_app    TEXT NOT NULL,
            account_role   TEXT NOT NULL,
            login_url      TEXT NOT NULL,
            username       TEXT NOT NULL,
            credential     TEXT NOT NULL,
            totp_secret    TEXT NOT NULL,
            backup_contact TEXT NOT NULL,
            remark         TEXT NOT NULL,
            extra_fields   TEXT NOT NULL DEFAULT '[]'
         );
         CREATE TABLE IF NOT EXISTS session_cookies (
            account_id TEXT PRIMARY KEY,
            data       TEXT NOT NULL,
            updated_at TEXT
         );
         CREATE TABLE IF NOT EXISTS platform_configs (
            platform          TEXT PRIMARY KEY,
            username_selector TEXT,
            password_selector TEXT,
            trigger_event     TEXT,
            verified          INTEGER,
            updated_at        TEXT
         );",
    )
    .map_err(|e| e.to_string())?;
    // 老账号库在解锁后的 prepare 阶段升级；解锁前的区域检查不写库。
    let mut stmt = conn.prepare("PRAGMA table_info(accounts)").map_err(|e| e.to_string())?;
    let columns = stmt.query_map([], |r| r.get::<_, String>(1)).map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
    if !columns.iter().any(|name| name == "extra_fields") {
        conn.execute("ALTER TABLE accounts ADD COLUMN extra_fields TEXT NOT NULL DEFAULT '[]'", [])
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 内置默认选择器，每次解锁时对一遍。
///
/// 选择器是照着各家登录页真实 DOM 抓的，不是猜的：
/// - 云账户 SSO（auth.yunzhanghu.com）：Ant Design，扫码/账号两套表单共用 name
/// - 即构（console.zego.im）：Element UI，输入框既没 id 也没 name，只能认 placeholder
/// - 微信公众平台（mp.weixin.qq.com）：扫码和账号密码都支持，所以不是 skip
const DEFAULTS: &[(&str, &str, &str, &str, bool)] = &[
    // 抓自 signin.aliyun.com/login.htm 真实 DOM
    ("阿里云", "#loginName", "#loginPassword", "input+change", true),
    // 腾讯云登录页有反爬，抓不到 DOM，这两条是照着实际页面的 placeholder 推的，
    // 所以标成未验证——不准就点「手动指认」重配一次，或用"在线适配"导入
    (
        "腾讯云",
        "input[placeholder*=邮箱地址]",
        "input[placeholder*=账号密码]",
        "input+change",
        false,
    ),
    (
        "云账户",
        "input[name=manage_name]",
        "input[name=password]",
        "input+change",
        true,
    ),
    (
        "即构",
        "input[placeholder='请输入邮箱/手机号']",
        "input[type=password]",
        "input+change",
        true,
    ),
    (
        "微信小程序",
        "input[name=account]",
        "input[name=password]",
        "input+change",
        true,
    ),
    ("其他", "", "", "", false),
];

/// 内置平台：每次解锁都会被补回来，删了也白删
pub fn is_builtin_platform(platform: &str) -> bool {
    DEFAULTS.iter().any(|d| d.0 == platform)
}

/// 把 `DEFAULTS` 补进库里。只填还没配过的行（选择器为空且未验证），用户手动指认或手改过的一律不碰——
/// 这样以后补默认值也能自动惠及老库，不会覆盖别人的成果。
fn seed_platform_configs(conn: &Connection) -> Result<(), String> {
    for (platform, u, p, ev, verified) in DEFAULTS {
        conn.execute(
            "INSERT INTO platform_configs
             (platform, username_selector, password_selector, trigger_event, verified, updated_at)
             VALUES (?1,?2,?3,?4,?5,'')
             ON CONFLICT(platform) DO UPDATE SET
               username_selector = excluded.username_selector,
               password_selector = excluded.password_selector,
               trigger_event     = excluded.trigger_event,
               verified          = excluded.verified
             WHERE platform_configs.verified = 0
               AND platform_configs.username_selector = ''
               AND platform_configs.password_selector = ''",
            params![platform, u, p, ev, *verified as i64],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn row_to_config(r: &rusqlite::Row) -> rusqlite::Result<PlatformConfig> {
    Ok(PlatformConfig {
        platform: r.get(0)?,
        username_selector: r.get(1)?,
        password_selector: r.get(2)?,
        trigger_event: r.get(3)?,
        verified: r.get::<_, i64>(4)? != 0,
        updated_at: r.get(5)?,
    })
}

pub fn list_platform_configs(conn: &Connection) -> Result<Vec<PlatformConfig>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT platform, username_selector, password_selector, trigger_event, verified, updated_at
             FROM platform_configs ORDER BY platform",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], row_to_config).map_err(|e| e.to_string())?;
    rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
}

pub fn get_platform_config(conn: &Connection, platform: &str) -> Result<Option<PlatformConfig>, String> {
    conn.query_row(
        "SELECT platform, username_selector, password_selector, trigger_event, verified, updated_at
         FROM platform_configs WHERE platform = ?1",
        params![platform],
        row_to_config,
    )
    .optional()
    .map_err(|e| e.to_string())
}

pub fn save_platform_config(conn: &Connection, c: &PlatformConfig) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO platform_configs
         (platform, username_selector, password_selector, trigger_event, verified, updated_at)
         VALUES (?1,?2,?3,?4,?5,?6)",
        params![
            c.platform,
            c.username_selector,
            c.password_selector,
            c.trigger_event,
            c.verified as i64,
            c.updated_at
        ],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// 会话 cookie 快照。存在加密库里而不是明文文件——这里面是登录凭证，
/// 跟密码一个级别，不能让拿到磁盘的人直接捡走。
pub fn save_cookies(conn: &Connection, account_id: &str, data: &str, now: &str) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO session_cookies (account_id, data, updated_at) VALUES (?1,?2,?3)",
        params![account_id, data, now],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

pub fn load_cookies(conn: &Connection, account_id: &str) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT data FROM session_cookies WHERE account_id = ?1",
        params![account_id],
        |r| r.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())
}

pub fn get_account(conn: &Connection, id: &str) -> Result<Account, String> {
    conn.query_row(
        "SELECT id, platform, owner_type, related_app, account_role, login_url,
                username, credential, totp_secret, backup_contact, remark, extra_fields
         FROM accounts WHERE id = ?1",
        params![id],
        row_to_account,
    )
    .map_err(|_| "账号不存在".to_string())
}

pub fn set_meta(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
        params![key, value],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

pub fn get_meta(conn: &Connection, key: &str) -> Result<String, String> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| r.get(0))
        .map_err(|e| e.to_string())
}

fn row_to_account(r: &rusqlite::Row) -> rusqlite::Result<Account> {
    Ok(Account {
        id: r.get(0)?,
        platform: r.get(1)?,
        owner_type: r.get(2)?,
        related_app: r.get(3)?,
        account_role: r.get(4)?,
        login_url: r.get(5)?,
        username: r.get(6)?,
        credential: r.get(7)?,
        totp_secret: r.get(8)?,
        backup_contact: r.get(9)?,
        remark: r.get(10)?,
        extra_fields: serde_json::from_str(&r.get::<_, String>(11)?)
            .map_err(|e| rusqlite::Error::FromSqlConversionFailure(11, rusqlite::types::Type::Text, Box::new(e)))?,
    })
}

pub fn list_accounts(conn: &Connection) -> Result<Vec<Account>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, platform, owner_type, related_app, account_role, login_url,
                    username, credential, totp_secret, backup_contact, remark, extra_fields
             FROM accounts ORDER BY platform, related_app, username",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], row_to_account).map_err(|e| e.to_string())?;
    rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
}

/// 导出加密备份：`VACUUM INTO` 出一份一致的副本（用主库同一把钥匙加密），再把 cookie 快照删掉。
///
/// cookie 快照是各控制台**活着的登录态**，备份文件会被拷到 U 盘、网盘、别的电脑，
/// 带着它等于随手散发一堆免密码、免两步验证的登录凭证。恢复后重新登一次就行，不值得冒这个险。
/// ATTACH 不带 KEY 时 SQLCipher 沿用主库的钥匙；删完 VACUUM 一遍，被删的数据不留在空闲页里。
pub fn export_backup(conn: &Connection, target: &Path) -> Result<(), String> {
    let target = target.to_string_lossy().to_string();
    conn.execute("VACUUM INTO ?1", [&target]).map_err(|e| e.to_string())?;
    conn.execute("ATTACH DATABASE ?1 AS bk", [&target]).map_err(|e| e.to_string())?;
    let cleaned = conn.execute_batch("DELETE FROM bk.session_cookies; VACUUM bk;").map_err(|e| e.to_string());
    let detached = conn.execute("DETACH DATABASE bk", []).map(|_| ()).map_err(|e| e.to_string());
    cleaned.and(detached)
}

/// 只数条数：验备份时用，老格式的库（没有 extra_fields 列）也数得了，而且不写库
pub fn count_accounts(conn: &Connection) -> Result<usize, String> {
    conn.query_row("SELECT count(*) FROM accounts", [], |r| r.get::<_, i64>(0))
        .map(|n| n as usize)
        .map_err(|_| "这份备份里没有账号表，不像是 Switchboard 的备份".to_string())
}

/// 新增或更新（前端负责生成 id，新增时是新 uuid）。
/// 顺手给这个平台建一条空的适配配置——平台是用户可以自由输入的，
/// 不这么做的话新平台在「平台适配配置」页里根本不出现，没法去配选择器。
pub fn save_account(conn: &Connection, a: &Account) -> Result<(), String> {
    let extra_fields = serde_json::to_string(&a.extra_fields).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT OR IGNORE INTO platform_configs
         (platform, username_selector, password_selector, trigger_event, verified, updated_at)
         VALUES (?1,'','','',0,'')",
        params![a.platform],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT OR REPLACE INTO accounts
         (id, platform, owner_type, related_app, account_role, login_url,
          username, credential, totp_secret, backup_contact, remark, extra_fields)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
        params![
            a.id,
            a.platform,
            a.owner_type,
            a.related_app,
            a.account_role,
            a.login_url,
            a.username,
            a.credential,
            a.totp_secret,
            a.backup_contact,
            a.remark,
            extra_fields
        ],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

pub fn delete_platform_config(conn: &Connection, platform: &str) -> Result<(), String> {
    conn.execute("DELETE FROM platform_configs WHERE platform = ?1", params![platform])
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub fn delete_cookies(conn: &Connection, account_id: &str) -> Result<(), String> {
    conn.execute("DELETE FROM session_cookies WHERE account_id = ?1", params![account_id])
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub fn delete_account(conn: &Connection, id: &str) -> Result<(), String> {
    delete_cookies(conn, id)?;
    conn.execute("DELETE FROM accounts WHERE id = ?1", params![id])
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ExtraField;

    fn sample() -> Account {
        Account {
            id: "a1".into(),
            platform: "阿里云".into(),
            owner_type: "公司账号".into(),
            related_app: "直播App - 推流服务".into(),
            account_role: "RAM子账号".into(),
            login_url: "https://signin.aliyun.com/".into(),
            username: "svc-live@company.com".into(),
            credential: "hunter2".into(),
            totp_secret: "".into(),
            backup_contact: "".into(),
            remark: "".into(),
            extra_fields: vec![ExtraField { label: "应用 ID".into(), value: "app-123".into(), secret: false }],
        }
    }

    #[test]
    fn unlock_precheck_does_not_write_before_prepare() {
        let path = std::env::temp_dir().join(format!("sb-precheck-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let conn = open_for_unlock(&path, "correct horse").unwrap();
        let tables: i64 = conn.query_row("SELECT count(*) FROM sqlite_master WHERE type='table'", [], |r| r.get(0)).unwrap();
        assert_eq!(tables, 0, "区域检查前不应建表或填入默认账号配置");
        prepare(&conn).unwrap();
        let tables: i64 = conn.query_row("SELECT count(*) FROM sqlite_master WHERE type='table'", [], |r| r.get(0)).unwrap();
        assert!(tables > 0);
        drop(conn);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn roundtrip_and_reject_wrong_password() {
        let path = std::env::temp_dir().join(format!("sb-test-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let conn = open(&path, "correct horse").unwrap();
        save_account(&conn, &sample()).unwrap();
        drop(conn);

        assert!(open(&path, "wrong horse").is_err(), "错密码必须被拒绝");

        let conn = open(&path, "correct horse").unwrap();
        assert_eq!(list_accounts(&conn).unwrap(), vec![sample()]);

        // 更新走同一条 save，不应该多出一行
        let mut edited = sample();
        edited.remark = "改过".into();
        save_account(&conn, &edited).unwrap();
        let all = list_accounts(&conn).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].remark, "改过");

        // cookie 快照要跟着账号一起删，不能留在库里
        save_cookies(&conn, "a1", "[\"k=v\"]", "now").unwrap();
        assert!(load_cookies(&conn, "a1").unwrap().is_some());
        delete_account(&conn, "a1").unwrap();
        assert!(list_accounts(&conn).unwrap().is_empty());
        assert!(load_cookies(&conn, "a1").unwrap().is_none());

        drop(conn);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn old_account_table_gains_extra_fields_without_losing_rows() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE accounts (
                id TEXT PRIMARY KEY, platform TEXT NOT NULL, owner_type TEXT NOT NULL,
                related_app TEXT NOT NULL, account_role TEXT NOT NULL, login_url TEXT NOT NULL,
                username TEXT NOT NULL, credential TEXT NOT NULL, totp_secret TEXT NOT NULL,
                backup_contact TEXT NOT NULL, remark TEXT NOT NULL
            );
            INSERT INTO accounts VALUES
                ('old', '即构', '团队', '直播', '主账号', 'https://example.com',
                 'name', 'password', '', '', '旧备注');",
        ).unwrap();

        prepare(&conn).unwrap();
        prepare(&conn).unwrap();
        let old = get_account(&conn, "old").unwrap();
        assert_eq!(old.remark, "旧备注");
        assert!(old.extra_fields.is_empty());

        let mut updated = old;
        updated.extra_fields.push(ExtraField { label: "邮箱".into(), value: "a@example.com".into(), secret: false });
        save_account(&conn, &updated).unwrap();
        assert_eq!(get_account(&conn, "old").unwrap().extra_fields, updated.extra_fields);
    }

    /// 落盘的库必须是真加密的，不能是任何人双击就能打开的明文 SQLite。
    /// 顺便钉住：空密码是被 SQLCipher 直接拒绝的（不会静默产出一个明文库）。
    #[test]
    fn database_on_disk_is_actually_encrypted() {
        let path = std::env::temp_dir().join(format!("sb-key-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let conn = open(&path, "real password").unwrap();
        save_account(&conn, &sample()).unwrap();
        drop(conn);

        let head = std::fs::read(&path).unwrap();
        assert!(
            !head.starts_with(b"SQLite format 3"),
            "库文件开头还是明文 SQLite 魔数，说明根本没加密"
        );
        assert!(
            !String::from_utf8_lossy(&head).contains("svc-live@company.com"),
            "账号名在文件里能直接搜到，说明根本没加密"
        );

        assert!(open(&path, "").is_err(), "空密码必须被拒绝，不能退化成明文库");

        let _ = std::fs::remove_file(&path);
    }

    /// 备份导出走 `VACUUM INTO`，副本必须还是用同一把主密码加密的完整库。
    #[test]
    fn backup_copy_keeps_the_same_key() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("sb-bk-{}.db", std::process::id()));
        let backup = dir.join(format!("sb-bk-{}-copy.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&backup);

        let conn = open(&path, "master pass 123").unwrap();
        save_account(&conn, &sample()).unwrap();
        conn.execute("VACUUM INTO ?1", [backup.to_string_lossy().to_string()])
            .unwrap();
        drop(conn);

        assert!(open(&backup, "some other pass").is_err(), "副本不该能被别的密码打开");
        let restored = open(&backup, "master pass 123").unwrap();
        assert_eq!(list_accounts(&restored).unwrap(), vec![sample()]);

        drop(restored);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&backup);
    }

    /// 导出的备份不带登录态：账号还在、同一把密码打得开、别的密码打不开，cookie 快照没了
    #[test]
    fn exported_backup_drops_session_cookies() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("sb-exp-{}.db", std::process::id()));
        let backup = dir.join(format!("sb-exp-{}-copy.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&backup);

        let conn = open(&path, "master pass 123").unwrap();
        save_account(&conn, &sample()).unwrap();
        save_cookies(&conn, &sample().id, r#"["sid=abc"]"#, "now").unwrap();
        export_backup(&conn, &backup).unwrap();
        assert!(load_cookies(&conn, &sample().id).unwrap().is_some(), "当前库自己的快照不能被删");
        drop(conn);

        assert!(open(&backup, "some other pass").is_err());
        let restored = open(&backup, "master pass 123").unwrap();
        assert_eq!(list_accounts(&restored).unwrap(), vec![sample()]);
        assert!(load_cookies(&restored, &sample().id).unwrap().is_none(), "备份里不该带登录态");

        drop(restored);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&backup);
    }

    /// 改主密码后，旧密码必须打不开，新密码必须打得开且数据还在。
    #[test]
    fn rekey_switches_the_master_password() {
        let path = std::env::temp_dir().join(format!("sb-rekey-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let conn = open(&path, "old password 1").unwrap();
        save_account(&conn, &sample()).unwrap();
        rekey(&conn, "new password 2").unwrap();
        drop(conn);

        assert!(open(&path, "old password 1").is_err(), "旧密码必须失效");
        let conn = open(&path, "new password 2").unwrap();
        assert_eq!(list_accounts(&conn).unwrap(), vec![sample()]);

        drop(conn);
        let _ = std::fs::remove_file(&path);
    }

    /// 用户自己输了个新平台，「平台适配配置」页必须能看到它（否则没法去配选择器）
    #[test]
    fn saving_an_account_registers_its_platform() {
        let path = std::env::temp_dir().join(format!("sb-newplat-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let conn = open(&path, "master pass 123").unwrap();
        let mut a = sample();
        a.platform = "知伴缘内部台".into();
        save_account(&conn, &a).unwrap();

        let cfg = get_platform_config(&conn, "知伴缘内部台").unwrap().unwrap();
        assert!(!cfg.verified, "新平台应该是待配置状态");
        assert!(list_platform_configs(&conn).unwrap().iter().any(|c| c.platform == "云账户"));

        drop(conn);
        let _ = std::fs::remove_file(&path);
    }

    /// 补默认值要能惠及"之前建好但还没配"的老行，否则用户得手动一个个填
    #[test]
    fn seed_fills_in_rows_that_were_left_unconfigured() {
        let path = std::env::temp_dir().join(format!("sb-upg-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let conn = open(&path, "master pass 123").unwrap();
        // 模拟老库：即构那行是空的
        save_platform_config(
            &conn,
            &PlatformConfig {
                platform: "即构".into(),
                username_selector: String::new(),
                password_selector: String::new(),
                trigger_event: String::new(),
                verified: false,
                updated_at: String::new(),
            },
        )
        .unwrap();
        drop(conn);

        // 重新解锁一次就该被补上
        let conn = open(&path, "master pass 123").unwrap();
        let zego = get_platform_config(&conn, "即构").unwrap().unwrap();
        assert!(zego.username_selector.contains("placeholder"), "即构应被补上默认选择器");
        assert!(zego.verified);

        let wx = get_platform_config(&conn, "微信小程序").unwrap().unwrap();
        assert_eq!(wx.username_selector, "input[name=account]");
        assert_ne!(wx.trigger_event, "skip", "微信公众平台支持账号密码，不该跳过填充");

        drop(conn);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn seeds_defaults_without_clobbering_user_edits() {
        let path = std::env::temp_dir().join(format!("sb-cfg-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let conn = open(&path, "master pass 123").unwrap();
        let ali = get_platform_config(&conn, "阿里云").unwrap().unwrap();
        assert_eq!(ali.username_selector, "#loginName");

        // 用户手动指认写回后，再次 open（每次解锁都会跑 seed）不能把它盖回默认值
        let mut edited = ali.clone();
        edited.username_selector = "input#ram-user".into();
        save_platform_config(&conn, &edited).unwrap();
        drop(conn);

        let conn = open(&path, "master pass 123").unwrap();
        assert_eq!(
            get_platform_config(&conn, "阿里云").unwrap().unwrap().username_selector,
            "input#ram-user"
        );

        drop(conn);
        let _ = std::fs::remove_file(&path);
    }
}
