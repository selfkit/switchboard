//! 按账号保存的字段绑定。远端只能报告定位/填写结果，值由后端逐条解析后定向注入。
use crate::{db, models::Account, session, totp, with_conn, Vault};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, Webview};
use tokio::sync::mpsc;

pub const AGENT: &str = include_str!("binding-agent.js");
fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Page {
    pub origin: String,
    pub pathname: String,
    pub hash_route: Option<String>,
    pub query_equals: BTreeMap<String, String>,
}
impl Page {
    fn matches(&self, other: &Self) -> bool {
        self.origin == other.origin
            && self.pathname == other.pathname
            && self.hash_route == other.hash_route
            && self.query_equals == other.query_equals
    }
    fn overlaps(&self, other: &Self) -> bool {
        self.matches(other)
    }
    fn validate(&self) -> Result<(), String> {
        let u: tauri::Url = self.origin.parse().map_err(|_| "页面地址无效")?;
        if !matches!(u.scheme(), "http" | "https")
            || u.origin().ascii_serialization() != self.origin
            || !self.pathname.starts_with('/')
            || self.pathname.len() > 2048
            || self.query_equals.len() > 30
            || self
                .hash_route
                .as_ref()
                .is_some_and(|h| !h.starts_with('/') || h.contains('?') || h.len() > 1024)
            || self.query_equals.iter().any(|(k, v)| k.len() > 100 || v.len() > 256)
        {
            return Err("页面匹配范围无效".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Locator {
    pub kind: String,
    pub value: String,
}
impl Locator {
    fn validate(&self) -> Result<(), String> {
        if self.kind != "css" || self.value.trim().is_empty() || self.value.len() > 2048 {
            return Err("只支持非空 CSS 定位".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum ContextStep {
    Shadow { locator: Locator },
    Frame { locator: Locator, page: Page },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Expected {
    pub tag: String,
    pub r#type: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Target {
    pub page: Page,
    pub context_path: Vec<ContextStep>,
    pub locator: Locator,
    pub expected: Expected,
}
impl Target {
    fn conflicts(&self, other: &Self) -> bool {
        self.locator == other.locator
            && self.page.overlaps(&other.page)
            && self.context_path.len() == other.context_path.len()
            && self
                .context_path
                .iter()
                .zip(&other.context_path)
                .all(|(a, b)| match (a, b) {
                    (ContextStep::Shadow { locator: a }, ContextStep::Shadow { locator: b }) => a == b,
                    (ContextStep::Frame { locator: a, page: ap }, ContextStep::Frame { locator: b, page: bp }) => {
                        a == b && ap.overlaps(bp)
                    }
                    _ => false,
                })
    }
    fn validate(&self) -> Result<(), String> {
        self.page.validate()?;
        self.locator.validate()?;
        if self.context_path.len() > 16 {
            return Err("输入框嵌套层数过多".into());
        }
        if !((self.expected.tag == "textarea" && self.expected.r#type.is_empty())
            || (self.expected.tag == "input"
                && matches!(
                    self.expected.r#type.as_str(),
                    "text" | "password" | "email" | "tel" | "url" | "search" | "number"
                )))
        {
            return Err("只支持文本输入框和 textarea".into());
        }
        let top: tauri::Url = self.page.origin.parse().map_err(|_| "页面地址无效")?;
        for step in &self.context_path {
            match step {
                ContextStep::Shadow { locator } => locator.validate()?,
                ContextStep::Frame { locator, page } => {
                    locator.validate()?;
                    page.validate()?;
                    let frame: tauri::Url = page.origin.parse().map_err(|_| "frame 地址无效")?;
                    if !session::fill_allowed(&top, &frame) {
                        return Err("暂不支持跨站 iframe 绑定".into());
                    }
                }
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum ValueRef {
    Account {
        field: String,
    },
    Totp,
    Extra {
        #[serde(rename = "fieldId")]
        field_id: String,
    },
}
impl ValueRef {
    fn validate(&self, a: &Account) -> Result<(), String> {
        match self {
            Self::Account { field } if matches!(field.as_str(), "username" | "password") => Ok(()),
            Self::Totp => Ok(()),
            Self::Extra { field_id } if a.extra_fields.iter().any(|f| &f.id == field_id) => Ok(()),
            _ => Err("missing_source".into()),
        }
    }
    fn resolve(&self, a: &Account) -> Result<String, String> {
        self.validate(a)?;
        let value = match self {
            Self::Account { field } if field == "username" => a.username.clone(),
            Self::Account { .. } => a.credential.clone(),
            Self::Totp => {
                let code = totp::generate(&a.totp_secret).map_err(|_| "empty_source")?;
                if code.remaining < 5 {
                    return Err("totp_wait".into());
                }
                code.code
            }
            Self::Extra { field_id } => a
                .extra_fields
                .iter()
                .find(|f| &f.id == field_id)
                .ok_or("missing_source")?
                .value
                .clone(),
        };
        if value.is_empty() {
            return Err("empty_source".into());
        }
        Ok(value)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Binding {
    pub schema_version: u8,
    pub id: String,
    pub account_id: String,
    pub label: String,
    pub enabled: bool,
    pub automatic: bool,
    pub value_ref: ValueRef,
    pub target: Target,
}
pub fn enabled(conn: &Connection, account: &str) -> bool {
    conn.query_row("SELECT 1 FROM binding_accounts WHERE account_id=?1", [account], |_| {
        Ok(())
    })
    .is_ok()
}
pub fn list(conn: &Connection, account: &str) -> Result<Vec<Binding>, String> {
    let mut s = conn
        .prepare("SELECT config FROM field_bindings WHERE account_id=?1 ORDER BY rowid")
        .map_err(|e| e.to_string())?;
    let json = s
        .query_map([account], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    json.into_iter()
        .map(|j| {
            let b: Binding = serde_json::from_str(&j).map_err(|_| "绑定配置损坏")?;
            if b.schema_version != 1 {
                return Err("unsupported_schema".into());
            }
            if b.account_id != account {
                return Err("绑定归属无效".into());
            }
            b.target.validate()?;
            Ok(b)
        })
        .collect()
}
fn persist(conn: &Connection, b: &Binding) -> Result<(), String> {
    b.target.validate()?;
    let account = db::get_account(conn, &b.account_id)?;
    if b.enabled {
        b.value_ref.validate(&account)?;
    }
    if b.label.trim().is_empty() || b.label.len() > 200 {
        return Err("绑定名称不能为空或过长".into());
    }
    let rows = list(conn, &b.account_id)?;
    if rows.len() >= 50 && !rows.iter().any(|x| x.id == b.id) {
        return Err("每个账号最多 50 条绑定".into());
    }
    if b.enabled
        && rows
            .iter()
            .any(|x| x.id != b.id && x.enabled && x.target.conflicts(&b.target))
    {
        return Err("这个输入框已有启用的绑定，请编辑原绑定".into());
    }
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    tx.execute(
        "INSERT INTO field_bindings(id,account_id,config) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET account_id=excluded.account_id, config=excluded.config",
        params![b.id, b.account_id, serde_json::to_string(b).map_err(|e| e.to_string())?],
    )
    .map_err(|e| e.to_string())?;
    tx.execute(
        "INSERT OR IGNORE INTO binding_accounts(account_id) VALUES(?1)",
        [&b.account_id],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    pub pick_id: String,
    pub target: Target,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemResult {
    pub binding_id: String,
    pub label: String,
    pub code: String,
    pub duration_ms: Option<u64>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub id: String,
    pub document_id: String,
    pub status: String,
    pub results: Vec<ItemResult>,
}
#[derive(Default)]
struct View {
    document: String,
    page: Option<Page>,
    armed: Option<Instant>,
    attempted: HashSet<String>,
    pick: Option<(String, Instant)>,
    draft: Option<Draft>,
    run: Option<Run>,
    sender: Option<mpsc::Sender<(String, String)>>,
    step: String,
}
#[derive(Default)]
pub struct Bindings(Mutex<HashMap<String, View>>);
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StateView {
    pub enabled: bool,
    pub bindings: Vec<Binding>,
    pub draft: Option<Draft>,
    pub picking: bool,
    pub run: Option<Run>,
    pub page: Option<Page>,
}
fn emit(app: &AppHandle) {
    let _ = app.emit_to(tauri::EventTarget::labeled("main"), "bindings-changed", ());
}
fn eval(app: &AppHandle, label: &str, data: serde_json::Value) -> Result<(), String> {
    app.get_webview(label)
        .ok_or("页面已关闭")?
        .eval(&format!(
            "window.__sbBindings && window.__sbBindings.command({});",
            data
        ))
        .map_err(|e| e.to_string())
}
fn unlocked(app: &AppHandle) -> bool {
    app.state::<Vault>().0.lock().is_ok_and(|v| v.is_some())
}

pub fn arm(app: &AppHandle, label: &str) {
    cancel(app, label, "stale_document");
    if let Ok(mut map) = app.state::<Bindings>().0.lock() {
        map.entry(label.into()).or_default().armed = Some(Instant::now());
    }
}
pub fn disarm(app: &AppHandle, label: &str) {
    cancel(app, label, "cancelled");
    if let Some(v) = app.get_webview(label) {
        let _ = v.eval("window.__sb && window.__sb.stopAutofill();");
    }
    if let Ok(mut map) = app.state::<Bindings>().0.lock() {
        map.entry(label.into()).or_default().armed = None;
    }
}
pub fn cancel(app: &AppHandle, label: &str, reason: &str) {
    if let Ok(mut map) = app.state::<Bindings>().0.lock() {
        if let Some(v) = map.get_mut(label) {
            if let Some(r) = &mut v.run {
                if r.status == "running" {
                    r.status = "cancelled".into();
                    for item in &mut r.results {
                        if item.code == "pending" {
                            item.code = reason.into();
                        }
                    }
                }
            }
            v.sender = None;
            v.step.clear();
            v.pick = None;
            v.draft = None;
        }
    }
    let _ = eval(app, label, serde_json::json!({"op":"cancel"}));
    emit(app);
}
pub fn cancel_all(app: &AppHandle) {
    let labels: Vec<_> = app
        .state::<Bindings>()
        .0
        .lock()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    for l in labels {
        disarm(app, &l);
    }
}
pub fn forget(app: &AppHandle, label: &str) {
    cancel(app, label, "cancelled");
    if let Ok(mut map) = app.state::<Bindings>().0.lock() {
        map.remove(label);
    }
}
pub fn navigation(app: &AppHandle, label: &str) {
    cancel(app, label, "stale_document");
    if let Ok(mut map) = app.state::<Bindings>().0.lock() {
        let v = map.entry(label.into()).or_default();
        v.document.clear();
        v.page = None;
        v.attempted.clear();
    }
}

pub fn state(app: &AppHandle, account: &str, label: &str) -> Result<StateView, String> {
    session::binding_context(app, account, label, false)?;
    let (mode, rows) = with_conn(&app.state::<Vault>(), |c| Ok((enabled(c, account), list(c, account)?)))?;
    let state = app.state::<Bindings>();
    let map = state.0.lock().map_err(|e| e.to_string())?;
    let v = map.get(label);
    Ok(StateView {
        enabled: mode,
        bindings: rows,
        draft: v.and_then(|v| v.draft.clone()),
        picking: v
            .and_then(|v| v.pick.as_ref())
            .is_some_and(|(_, t)| t.elapsed().as_secs() < 60),
        run: v.and_then(|v| v.run.clone()),
        page: v.and_then(|v| v.page.clone()),
    })
}
pub fn pick(app: &AppHandle, account: &str, label: &str) -> Result<(), String> {
    if !unlocked(app) {
        return Err("账号库未解锁".into());
    }
    session::binding_context(app, account, label, true)?;
    session::set_picking(app, account, false)?;
    if let Some(v) = app.get_webview(label) {
        let _ = v.eval("window.__sb && window.__sb.stopAutofill();");
    }
    cancel(app, label, "cancelled");
    let pick_id = id();
    let document = {
        let state = app.state::<Bindings>();
        let mut m = state.0.lock().map_err(|e| e.to_string())?;
        let v = m.entry(label.into()).or_default();
        if v.document.is_empty() {
            return Err("页面绑定工具尚未就绪，请稍后重试".into());
        }
        v.pick = Some((pick_id.clone(), Instant::now()));
        v.document.clone()
    };
    eval(
        app,
        label,
        serde_json::json!({"op":"pick","pickId":pick_id,"document":document}),
    )?;
    emit(app);
    Ok(())
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Save {
    pub id: Option<String>,
    pub pick_id: Option<String>,
    pub label: String,
    pub enabled: bool,
    pub automatic: bool,
    pub value_ref: ValueRef,
}
pub fn save(app: &AppHandle, account: &str, label: &str, input: Save) -> Result<String, String> {
    session::binding_context(app, account, label, true)?;
    let draft = state(app, account, label)?.draft;
    let result = with_conn(&app.state::<Vault>(), |c| {
        let old = if let Some(id) = &input.id {
            Some(
                list(c, account)?
                    .into_iter()
                    .find(|b| &b.id == id)
                    .ok_or("绑定不存在")?,
            )
        } else {
            None
        };
        let target = if let Some(pick) = &input.pick_id {
            draft
                .as_ref()
                .filter(|d| &d.pick_id == pick)
                .ok_or("指认已失效，请重新选择")?
                .target
                .clone()
        } else {
            old.as_ref().ok_or("请先选择输入框")?.target.clone()
        };
        let b = Binding {
            schema_version: 1,
            id: old.map(|b| b.id).unwrap_or_else(id),
            account_id: account.into(),
            label: input.label,
            enabled: input.enabled,
            automatic: input.automatic,
            value_ref: input.value_ref,
            target,
        };
        persist(c, &b)?;
        Ok(b.id)
    })?;
    cancel_account(app, account);
    // 保存只改变配置；当前文档的周期性 hello 不能把“保存”变成“试填”。
    let labels = session::binding_labels(app, account);
    if let Ok(mut m) = app.state::<Bindings>().0.lock() {
        for label in labels {
            if let Some(v) = m.get_mut(&label) {
                v.attempted.insert(result.clone());
            }
        }
    }
    emit(app);
    Ok(result)
}
pub fn remove(app: &AppHandle, account: &str, binding: &str) -> Result<(), String> {
    with_conn(&app.state::<Vault>(), |c| {
        c.execute(
            "DELETE FROM field_bindings WHERE id=?1 AND account_id=?2",
            params![binding, account],
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    })?;
    cancel_account(app, account);
    Ok(())
}
pub fn mode(app: &AppHandle, account: &str, on: bool) -> Result<(), String> {
    with_conn(&app.state::<Vault>(), |c| {
        db::get_account(c, account)?;
        c.execute(
            if on {
                "INSERT OR IGNORE INTO binding_accounts(account_id) VALUES(?1)"
            } else {
                "DELETE FROM binding_accounts WHERE account_id=?1"
            },
            [account],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })?;
    cancel_account(app, account);
    for label in session::binding_labels(app, account) {
        disarm(app, &label);
    }
    Ok(())
}
fn cancel_account(app: &AppHandle, account: &str) {
    for label in session::binding_labels(app, account) {
        cancel(app, &label, "cancelled");
        if let Some(v) = app.get_webview(&label) {
            let _ = v.eval("window.__sb && window.__sb.stopAutofill && window.__sb.stopAutofill();");
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Event {
    Document {
        document: String,
        page: Page,
    },
    Picked {
        document: String,
        #[serde(rename = "pickId")]
        pick_id: String,
        target: Target,
    },
    PickCancelled {
        document: String,
        #[serde(rename = "pickId")]
        pick_id: String,
    },
    Reply {
        document: String,
        #[serde(rename = "runId")]
        run_id: String,
        #[serde(rename = "stepId")]
        step_id: String,
        code: String,
    },
}
/// 远端报告命令没有返回字段值。所有身份来自 Tauri 提供的 Webview，而不是页面载荷。
pub fn report(app: &AppHandle, webview: &Webview, event: Event) -> Result<(), String> {
    let label = webview.label();
    let account = session::binding_account(app, label).ok_or("未知页面")?;
    let native = session::binding_context(app, &account, label, true)?;
    if !unlocked(app) {
        return Err("账号库未解锁".into());
    }
    match event {
        Event::Document { document, page } => {
            page.validate()?;
            if document.len() > 128 || document.is_empty() || page.origin != native.origin().ascii_serialization() {
                return Err("页面来源不匹配".into());
            }
            let changed = app
                .state::<Bindings>()
                .0
                .lock()
                .map_err(|e| e.to_string())?
                .get(label)
                .map_or(true, |v| v.document != document);
            // 页面脚本每 30 秒会发一次心跳确认"还是这个文档"。没变就什么都不做：
            // 以前这里每次都跑一遍自动填写查询、再推一次 bindings-changed，
            // 等于每个保活页面每分钟白查好几次库、让前端跟着重渲染。
            if !changed {
                // 指认有 60 秒有效期；心跳顺便把过期的指认清掉，但别的什么都不做。
                if let Ok(mut m) = app.state::<Bindings>().0.lock() {
                    if let Some(v) = m.get_mut(label) {
                        if v.pick.as_ref().is_some_and(|(_, t)| t.elapsed().as_secs() >= 60) {
                            v.pick = None;
                        }
                    }
                }
                return Ok(());
            }
            cancel(app, label, "stale_document");
            {
                let state = app.state::<Bindings>();
                let mut m = state.0.lock().map_err(|e| e.to_string())?;
                let v = m.entry(label.into()).or_default();
                v.attempted.clear();
                if v.pick.as_ref().is_some_and(|(_, t)| t.elapsed().as_secs() >= 60) {
                    v.pick = None;
                }
                v.document = document;
                v.page = Some(page);
            }
            let _ = start(app, &account, label, None, false, true);
        }
        Event::Picked {
            document,
            pick_id,
            target,
        } => {
            target.validate()?;
            let state = app.state::<Bindings>();
            let mut m = state.0.lock().map_err(|e| e.to_string())?;
            let v = m.get_mut(label).ok_or("指认已取消")?;
            if v.document != document
                || !v
                    .pick
                    .as_ref()
                    .is_some_and(|(id, t)| id == &pick_id && t.elapsed().as_secs() < 60)
                || !v.page.as_ref().is_some_and(|p| target.page == *p)
            {
                return Err("指认已失效".into());
            }
            v.draft = Some(Draft { pick_id, target });
            v.pick = None;
        }
        Event::PickCancelled { document, pick_id } => {
            let state = app.state::<Bindings>();
            let mut m = state.0.lock().map_err(|e| e.to_string())?;
            if let Some(v) = m.get_mut(label) {
                if v.document == document && v.pick.as_ref().is_some_and(|(p, _)| p == &pick_id) {
                    v.pick = None;
                }
            }
        }
        Event::Reply {
            document,
            run_id,
            step_id,
            code,
        } => {
            if !matches!(
                code.as_str(),
                "ready"
                    | "filled"
                    | "already_matches"
                    | "nonempty_skipped"
                    | "target_not_found"
                    | "ambiguous_target"
                    | "unsupported_target"
                    | "value_rejected"
                    | "page_mismatch"
                    | "frame_mismatch"
                    | "stale_document"
                    | "timeout"
            ) {
                return Err("未知填写结果".into());
            }
            let state = app.state::<Bindings>();
            let m = state.0.lock().map_err(|e| e.to_string())?;
            let v = m.get(label).ok_or("任务已结束")?;
            if v.document != document
                || v.step != step_id
                || !v.run.as_ref().is_some_and(|r| r.id == run_id && r.status == "running")
            {
                return Err("任务已失效".into());
            }
            if let Some(tx) = &v.sender {
                let _ = tx.try_send((step_id, code));
            }
        }
    }
    emit(app);
    Ok(())
}

pub fn start(
    app: &AppHandle,
    account: &str,
    label: &str,
    binding: Option<String>,
    overwrite: bool,
    automatic: bool,
) -> Result<String, String> {
    session::binding_context(app, account, label, true)?;
    let rows = with_conn(&app.state::<Vault>(), |c| {
        if !enabled(c, account) {
            return Err("请先启用字段绑定模式".into());
        }
        list(c, account)
    })?;
    let run_id = id();
    let (document, rows, rx) = {
        let state = app.state::<Bindings>();
        let mut m = state.0.lock().map_err(|e| e.to_string())?;
        let v = m.get_mut(label).ok_or("页面绑定工具尚未就绪")?;
        if v.run.as_ref().is_some_and(|r| r.status == "running") {
            return Err("已有填写任务，请等待或取消".into());
        }
        if automatic && (v.pick.is_some() || v.draft.is_some() || !v.armed.is_some_and(|t| t.elapsed().as_secs() < 120))
        {
            return Err("自动填写未授权".into());
        }
        let page = v.page.as_ref().ok_or("页面尚未就绪")?;
        let selected: Vec<_> = rows
            .into_iter()
            .filter(|b| {
                b.enabled
                    && b.target.page.matches(page)
                    && binding.as_ref().map_or(true, |id| &b.id == id)
                    && (!automatic || (b.automatic && !v.attempted.contains(&b.id)))
            })
            .collect();
        if selected.is_empty() {
            return Err("当前页面没有可执行的绑定".into());
        }
        let (tx, rx) = mpsc::channel(16);
        for b in &selected {
            v.attempted.insert(b.id.clone());
        }
        v.run = Some(Run {
            id: run_id.clone(),
            document_id: v.document.clone(),
            status: "running".into(),
            results: selected
                .iter()
                .map(|b| ItemResult {
                    binding_id: b.id.clone(),
                    label: b.label.clone(),
                    code: "pending".into(),
                    duration_ms: None,
                })
                .collect(),
        });
        v.sender = Some(tx);
        (v.document.clone(), selected, rx)
    };
    let (handle, account, label, rid) = (app.clone(), account.to_string(), label.to_string(), run_id.clone());
    tauri::async_runtime::spawn(async move {
        execute(handle, account, label, rid, document, rows, overwrite && !automatic, rx).await;
    });
    emit(app);
    Ok(run_id)
}
fn running(app: &AppHandle, label: &str, rid: &str, document: &str) -> bool {
    app.state::<Bindings>().0.lock().is_ok_and(|m| {
        m.get(label).is_some_and(|v| {
            v.document == document && v.run.as_ref().is_some_and(|r| r.id == rid && r.status == "running")
        })
    })
}
async fn receive(rx: &mut mpsc::Receiver<(String, String)>, step: &str, deadline: tokio::time::Instant) -> String {
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Some((id, code))) if id == step => return code,
            Ok(Some(_)) => continue,
            Ok(None) => return "cancelled".into(),
            Err(_) => return "timeout".into(),
        }
    }
}
async fn execute(
    app: AppHandle,
    account: String,
    label: String,
    rid: String,
    document: String,
    rows: Vec<Binding>,
    overwrite: bool,
    mut rx: mpsc::Receiver<(String, String)>,
) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    for b in rows {
        let since = Instant::now();
        if !running(&app, &label, &rid, &document) {
            return;
        }
        let step = id();
        if let Ok(mut m) = app.state::<Bindings>().0.lock() {
            if let Some(v) = m.get_mut(&label) {
                v.step = step.clone();
            }
        }
        let mut code = if eval(&app,&label,serde_json::json!({"op":"locate","runId":rid,"stepId":step,"document":document,"target":b.target,"overwrite":overwrite})).is_err() { "cancelled".into() }
            else { receive(&mut rx,&step,deadline).await };
        if code == "ready" {
            loop {
                if !running(&app, &label, &rid, &document) {
                    return;
                }
                if tokio::time::Instant::now() >= deadline {
                    code = "timeout".into();
                    break;
                }
                let value = with_conn(&app.state::<Vault>(), |c| {
                    if !enabled(c, &account) {
                        return Err("cancelled".into());
                    }
                    let fresh = list(c, &account)?
                        .into_iter()
                        .find(|x| x.id == b.id && x.enabled)
                        .ok_or("cancelled")?;
                    if fresh.target != b.target || fresh.value_ref != b.value_ref {
                        return Err("cancelled".into());
                    }
                    fresh.value_ref.resolve(&db::get_account(c, &account)?)
                });
                match value {
                    Err(e) if e == "totp_wait" => {
                        tokio::time::sleep(Duration::from_millis(250)).await;
                        continue;
                    }
                    Err(e) => {
                        code = if e == "账号库未解锁" {
                            "vault_locked".into()
                        } else if matches!(e.as_str(), "missing_source" | "empty_source" | "cancelled") {
                            e
                        } else {
                            "missing_source".into()
                        };
                    }
                    Ok(value) => {
                        if !running(&app, &label, &rid, &document) || !unlocked(&app) {
                            return;
                        }
                        if session::binding_context(&app,&account,&label,true).is_err() { code="session_cut".into(); }
                        else if !app.get_webview(&label).and_then(|v|v.url().ok()).is_some_and(|u|u.origin().ascii_serialization()==b.target.page.origin) { code="page_mismatch".into(); }
                        else if eval(&app,&label,serde_json::json!({"op":"apply","runId":rid,"stepId":step,"document":document,"value":value})).is_err() { code="cancelled".into(); }
                        else {
                            code=receive(&mut rx,&step,deadline).await;
                            // 重复 ready 不是填写成功，只有最终回执才算。
                            while code=="ready" { code=receive(&mut rx,&step,deadline).await; }
                        }
                    }
                }
                break;
            }
        }
        if !running(&app, &label, &rid, &document) {
            return;
        }
        if let Ok(mut m) = app.state::<Bindings>().0.lock() {
            if let Some(v) = m.get_mut(&label) {
                if let Some(r) = &mut v.run {
                    if r.id == rid {
                        if let Some(item) = r.results.iter_mut().find(|i| i.binding_id == b.id) {
                            item.code = code;
                            item.duration_ms = Some(since.elapsed().as_millis() as u64);
                        }
                    }
                }
            }
        }
        emit(&app);
    }
    if let Ok(mut m) = app.state::<Bindings>().0.lock() {
        if let Some(v) = m.get_mut(&label) {
            if let Some(r) = &mut v.run {
                if r.id == rid && r.status == "running" {
                    let ok = r
                        .results
                        .iter()
                        .filter(|i| matches!(i.code.as_str(), "filled" | "already_matches"))
                        .count();
                    r.status = if ok == r.results.len() {
                        "completed"
                    } else if ok > 0 {
                        "partial"
                    } else {
                        "failed"
                    }
                    .into();
                    v.sender = None;
                    v.step.clear();
                }
            }
        }
    }
    let _ = eval(&app, &label, serde_json::json!({"op":"finish","runId":rid}));
    emit(&app);
}

#[cfg(test)]
mod tests {
    use super::*;
    fn account() -> Account {
        serde_json::from_value(serde_json::json!({
            "id":"a", "platform":"其他", "owner_type":"", "related_app":"", "account_role":"", "login_url":"https://example.com/login", "username":"alice", "credential":"pw", "totp_secret":"", "backup_contact":"", "remark":"",
            "extra_fields":[{"id":"extra-a", "label":"编号", "value":"tenant-1", "secret":true}]
        })).unwrap()
    }
    fn binding() -> Binding {
        serde_json::from_value(serde_json::json!({
            "schemaVersion":1,"id":"binding-a","accountId":"a","label":"企业编号","enabled":true,"automatic":false,
            "valueRef":{"kind":"extra","fieldId":"extra-a"},
            "target":{"page":{"origin":"https://example.com","pathname":"/login","hashRoute":null,"queryEquals":{}},"contextPath":[],"locator":{"kind":"css","value":"#tenant"},"expected":{"tag":"input","type":"text"}}
        })).unwrap()
    }
    fn setup() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        db::prepare(&c).unwrap();
        db::save_account(&c, &account()).unwrap();
        c
    }
    #[test]
    fn scope_requires_origin_path_hash_and_selected_queries() {
        let mut p = binding().target.page;
        let mut other = p.clone();
        other.pathname = "/reset-password".into();
        assert!(!p.matches(&other));
        other = p.clone();
        other.origin = "https://other.example.com".into();
        assert!(!p.matches(&other));
        other = p.clone();
        other.hash_route = Some("/reset".into());
        assert!(!p.matches(&other));
        p.query_equals.insert("tenant".into(), "one".into());
        other = p.clone();
        assert!(p.matches(&other));
        other.query_equals.insert("tenant".into(), "two".into());
        assert!(!p.matches(&other));
        other = p.clone();
        other.query_equals.insert("lang".into(), "zh".into());
        assert!(!p.matches(&other));
    }
    #[test]
    fn rejects_unsupported_context_controls_and_schema() {
        let mut b = binding();
        b.target.expected.r#type = "file".into();
        assert!(b.target.validate().is_err());
        b = binding();
        b.target.context_path.push(ContextStep::Frame {
            locator: Locator {
                kind: "css".into(),
                value: "#f".into(),
            },
            page: Page {
                origin: "https://evil.test".into(),
                ..b.target.page.clone()
            },
        });
        assert!(b.target.validate().is_err());
        b = binding();
        b.target.locator.kind = "javascript".into();
        assert!(b.target.validate().is_err());
        let c = setup();
        b = binding();
        b.schema_version = 2;
        c.execute(
            "INSERT INTO field_bindings VALUES(?1,?2,?3)",
            params![b.id, b.account_id, serde_json::to_string(&b).unwrap()],
        )
        .unwrap();
        assert_eq!(list(&c, "a").unwrap_err(), "unsupported_schema");
    }
    #[test]
    fn source_is_stable_through_rename_reorder_and_reads_latest_value() {
        let mut a = account();
        let source = binding().value_ref;
        a.extra_fields[0].label = "新名称".into();
        a.extra_fields[0].value = "new-value".into();
        a.extra_fields.insert(
            0,
            crate::models::ExtraField {
                id: "another".into(),
                label: "编号".into(),
                value: "wrong".into(),
                secret: false,
            },
        );
        assert_eq!(source.resolve(&a).unwrap(), "new-value");
        a.extra_fields.remove(1);
        assert_eq!(source.resolve(&a).unwrap_err(), "missing_source");
        a.credential.clear();
        assert_eq!(
            ValueRef::Account {
                field: "password".into()
            }
            .resolve(&a)
            .unwrap_err(),
            "empty_source"
        );
        assert_eq!(ValueRef::Totp.resolve(&a).unwrap_err(), "empty_source");
    }
    #[test]
    fn bindings_are_account_scoped_reject_duplicate_targets_and_cascade_delete() {
        let c = setup();
        let b = binding();
        persist(&c, &b).unwrap();
        assert!(enabled(&c, "a"));
        assert_eq!(list(&c, "a").unwrap().len(), 1);
        let mut other = b.clone();
        other.id = "other".into();
        assert!(persist(&c, &other).is_err());
        other.enabled = false;
        persist(&c, &other).unwrap();
        assert_eq!(list(&c, "a").unwrap().len(), 2);
        assert!(list(&c, "other-account").unwrap().is_empty());
        assert!(!enabled(&c, "other-account"));
        db::delete_account(&c, "a").unwrap();
        assert!(list(&c, "a").unwrap().is_empty());
        assert!(!enabled(&c, "a"));
    }
    #[test]
    fn deleted_source_is_visible_but_cannot_run_and_can_be_disabled() {
        let c = setup();
        let mut b = binding();
        persist(&c, &b).unwrap();
        let mut a = account();
        a.extra_fields.clear();
        db::save_account(&c, &a).unwrap();
        assert_eq!(list(&c, "a").unwrap().len(), 1);
        assert!(b.value_ref.resolve(&a).is_err());
        b.enabled = false;
        persist(&c, &b).unwrap();
        b.enabled = true;
        assert!(persist(&c, &b).is_err());
        assert!(!serde_json::to_string(&list(&c, "a").unwrap())
            .unwrap()
            .contains("tenant-1"));
    }
}
