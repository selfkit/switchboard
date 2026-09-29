import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import * as api from "../api";
import type { Account, ExtraField } from "../api";
import { Button, C, Card, Combo, ErrorLine, Field, HelpDot, IS_MAC, inputStyle } from "../ui";

type Props = {
  initial: Account;
  isNew: boolean;
  copiedFrom?: string;
  /** 用来算候选值：已经存过的平台/归属/角色/URL 都会出现在下拉里 */
  accounts: Account[];
  onSave: (a: Account) => Promise<void>;
  onCancel: () => void;
};

/** AddAccount.dc.html，新增与编辑共用 */
export default function AccountForm({ initial, isNew, copiedFrom, accounts, onSave, onCancel }: Props) {
  const [a, setA] = useState(initial);
  const [err, setErr] = useState("");
  const [showHelp, setShowHelp] = useState(false);
  const set = (k: keyof Account) => (e: { target: { value: string } }) => setA({ ...a, [k]: e.target.value });
  const put = (k: keyof Account) => (v: string) => setA({ ...a, [k]: v });
  const updateExtra = (index: number, patch: Partial<ExtraField>) => setA((prev) => ({
    ...prev,
    extra_fields: prev.extra_fields.map((field, i) => i === index ? { ...field, ...patch } : field),
  }));
  const opts = api.fieldOptions(accounts, a.platform);
  const [totpErr, setTotpErr] = useState("");
  const [preview, setPreview] = useState<{ code: string; remaining: number } | null>(null);

  // 填了密钥就当场出码：跟手机验证器上的对一下，一致就说明密钥没抄错
  useEffect(() => {
    const secret = a.totp_secret.trim();
    setPreview(null);
    if (!secret) return setTotpErr("");
    const tick = () => api.totpPreview(secret).then((c) => { setPreview(c); setTotpErr(""); }).catch((e) => { setPreview(null); setTotpErr(String(e)); });
    tick();
    const t = window.setInterval(tick, 1000);
    return () => window.clearInterval(t);
  }, [a.totp_secret]);

  const totpFrom = (src: { text?: string; path?: string }) =>
    api.totpSecretFrom(src).then((secret) => setA((prev) => ({ ...prev, totp_secret: secret }))).catch((e) => setTotpErr(String(e)));

  async function submit() {
    if (!a.username.trim()) return setErr("登录名不能为空");
    const extra_fields = a.extra_fields.filter((field) => field.label.trim() || field.value);
    if (extra_fields.some((field) => !field.label.trim() || !field.value)) {
      return setErr("附加字段的名称和值需要一起填写");
    }
    try {
      await onSave({ ...a, extra_fields: extra_fields.map((field) => ({ ...field, label: field.label.trim() })) });
    } catch (e) {
      setErr(String(e));
    }
  }

  const two: React.CSSProperties = { display: "grid", gridTemplateColumns: "repeat(2, minmax(0, 1fr))", gap: 16 };

  return (
    <div style={{ flexGrow: 1, minWidth: 0, display: "flex", flexDirection: "column", background: C.bg }}>
      <div style={{ flexGrow: 1, minHeight: 0, overflowY: "auto", display: "flex", justifyContent: "center" }}>
        <div style={{ width: 680, padding: "8px 0 24px", display: "flex", flexDirection: "column", gap: 20 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
          <h1 style={{ margin: 0, fontWeight: 500, fontSize: 24, color: C.text }}>{copiedFrom ? "复制新建账号" : isNew ? "新增账号" : "编辑账号"}</h1>
          <HelpDot on={showHelp} onClick={() => setShowHelp(!showHelp)} title="「一键登录」是怎么工作的" />
        </div>
        <div style={{ fontSize: 13, color: C.sub, marginTop: -12 }}>每条记录会自动获得独立的 cookie 存储，多个账号同时在线互不冲突</div>
        {copiedFrom && (
          <div style={{ fontSize: 12, color: C.sub, padding: "10px 12px", borderRadius: 7, background: C.brandSoft }}>
            已从“{copiedFrom}”预填全部字段（包括密码、TOTP 密钥和附加字段）。请核对并修改后保存，新账号会使用独立的登录会话。
          </div>
        )}

        {showHelp && (
        <Card style={{ display: "flex", flexDirection: "column", gap: 8, background: C.brandSoft, border: "none" }}>
          <div style={{ fontSize: 13, color: C.text, fontWeight: 500 }}>「一键登录」是怎么工作的</div>
          <div style={{ fontSize: 12, color: C.sub, lineHeight: 1.8 }}>
            1. <b>跳到哪</b>：只看下面的「登录直达 URL」，跟平台没关系。填什么就打开什么。
            <br />
            2. <b>自动填账号密码</b>：按「平台」去查<b>平台适配配置</b>里存的输入框选择器。
            没适配过的平台照样能打开，只是不会自动填。
            <br />
            3. <b>没自动填上怎么办</b>：到「会话」页，点一下页面上的输入框，再点左边的
            「填账号 / 填密码 / 填附加字段」；想让它以后自己认得账号和密码，点一次「指认输入框」。
            <br />
            4. <b>登录直达 URL 填哪个</b>：那个<b>真正带账号密码输入框</b>的页面，不是官网首页——
            很多平台的登录页在另一个域名下（比如云账户在 auth.yunzhanghu.com，不在 open.yunzhanghu.com）。
            不知道准确地址就先随便填个能进的，打开后在「会话」页点到登录页，再点左边的「设为直达页」。
          </div>
        </Card>
        )}

        <Section title="身份" desc="这条记录是谁、属于哪个平台">
          <div style={two}>
            <Field label="平台">
              <Combo value={a.platform} onChange={put("platform")} options={opts.platforms} placeholder="选择或直接输入" />
            </Field>
            <Field label="登录名 / 邮箱">
              <input style={inputStyle} placeholder="svc-live@company.com" value={a.username} onChange={set("username")} />
            </Field>
          </div>
          <div style={two}>
            <Field label="归属">
              <Combo value={a.owner_type} onChange={put("owner_type")} options={opts.owners} placeholder="选择或直接输入" />
            </Field>
            <Field label="账号角色">
              <Combo value={a.account_role} onChange={put("account_role")} options={opts.roles} placeholder="选择或直接输入" />
            </Field>
          </div>
        </Section>

        <Section title="登录" desc="一键登录要用到的地址和凭据">
          <Field label="登录直达 URL" hint="填带账号密码输入框的那个页面；不确定就点右上角问号看说明">
            <Combo value={a.login_url} onChange={put("login_url")} options={opts.urls} placeholder="https://xxx.console.aliyun.com/..." />
          </Field>
          <Field label="密码（加密存储）">
            <input style={inputStyle} type="password" placeholder="••••••••" value={a.credential} onChange={set("credential")} />
          </Field>
          <Field
            label="TOTP 密钥（可选）"
            hint={
              <>
                作用：登录要两步验证码时不用掏手机，这里直接出码、会话页一键填入。原理是验证码由「密钥 + 当前时间」算出来，
                手机验证器（Google / 微软 / 小程序）存的也是这个密钥，两边拿到同一个就出同样的码，手机那边照常能用。
                <br />
                密钥在哪：平台绑定两步验证（虚拟 MFA）时，二维码旁的「无法扫码 / 手动输入」里那串字母数字就是；
                {/* 认二维码图片用的是 macOS 自带的 CoreImage，别的系统只收 otpauth:// 链接 */}
                {IS_MAC
                  ? "也可以把二维码截图（⌘⌃⇧4）或 otpauth:// 链接直接粘贴到上面的输入框。"
                  : "也可以把 otpauth:// 链接直接粘贴到上面的输入框（这台电脑上不支持直接识别二维码图片）。"}
                已经绑好的账号平台不会再显示密钥，要解绑后重新绑定一次。
                <br />
                用不了的：短信 / 邮件验证码、微信扫码确认、App 里点「确认登录」这类推送——它们不是 TOTP，没有密钥。
              </>
            }
          >
            <div style={{ display: "flex", gap: 8 }}>
              <input
                style={inputStyle}
                placeholder={IS_MAC ? "Base32 密钥，或粘贴二维码截图 / otpauth:// 链接" : "Base32 密钥，或粘贴 otpauth:// 链接"}
                value={a.totp_secret}
                onChange={set("totp_secret")}
                onPaste={(e) => {
                  // otpauth 链接拦下来取密钥；剪贴板里是图片（或者没有文字，多半是截图）交给后端认二维码；
                  // 其余照常粘贴。浏览器里"拷贝图像"会顺带一段网址文字，所以有图片就按图片认
                  const text = e.clipboardData.getData("text").trim();
                  const isOtpauth = text.startsWith("otpauth");
                  // 截图只有 macOS 认得了；别的系统上贴图片就当普通粘贴，什么都不做
                  const isImage = IS_MAC && (!text || Array.from(e.clipboardData.items).some((i) => i.type.startsWith("image/")));
                  if (!isOtpauth && !isImage) return;
                  e.preventDefault();
                  setTotpErr("");
                  void totpFrom(isOtpauth ? { text } : {});
                }}
              />
              {IS_MAC && (
                <Button
                  onClick={async () => {
                    const path = await open({ multiple: false, filters: [{ name: "二维码图片", extensions: ["png", "jpg", "jpeg", "heic", "tiff", "gif", "bmp", "webp"] }] });
                    if (typeof path === "string") void totpFrom({ path });
                  }}
                  style={{ flexShrink: 0 }}
                >
                  选二维码图片
                </Button>
              )}
            </div>
            {preview && (
              <div style={{ fontSize: 12, color: C.sub }}>
                当前验证码 <b style={{ color: C.text, fontFamily: "ui-monospace, monospace", letterSpacing: 2 }}>{preview.code.slice(0, 3)} {preview.code.slice(3)}</b>
                <span style={{ color: C.muted }}>（{preview.remaining} 秒后刷新）· 跟手机验证器上的一致，就说明密钥对了</span>
              </div>
            )}
            <ErrorLine text={totpErr} />
          </Field>
        </Section>

        <Section title="备注" desc="方便以后找回来的信息，都可以不填">
          <div style={two}>
            <Field label="关联应用 / 项目">
              <input style={inputStyle} placeholder="例如：直播App - 推流服务" value={a.related_app} onChange={set("related_app")} />
            </Field>
            <Field label="验证码联系人">
              <input style={inputStyle} placeholder="例如：财务-张三 手机尾号1234" value={a.backup_contact} onChange={set("backup_contact")} />
            </Field>
          </div>
          <Field label="备注">
            <textarea style={{ ...inputStyle, resize: "vertical" }} rows={2} placeholder="其他说明" value={a.remark} onChange={set("remark")} />
          </Field>
        </Section>

        <Section
          title="附加字段"
          desc="邮箱、应用 ID、主体等；会话页可一键填入网页输入框"
          action={<Button onClick={() => setA((prev) => ({ ...prev, extra_fields: [...prev.extra_fields, { label: "", value: "", secret: false }] }))}>+ 添加字段</Button>}
        >
          {a.extra_fields.length === 0 && <div style={{ fontSize: 12, color: C.muted }}>还没有附加字段</div>}
          {a.extra_fields.map((field, index) => (
            <div key={index} style={{ display: "grid", gridTemplateColumns: "minmax(100px, 1fr) minmax(140px, 2fr) auto auto", alignItems: "center", gap: 8 }}>
              <input style={inputStyle} placeholder="名称，例如：邮箱" value={field.label} onChange={(e) => updateExtra(index, { label: e.target.value })} />
              <input style={inputStyle} type={field.secret ? "password" : "text"} placeholder="内容" value={field.value} onChange={(e) => updateExtra(index, { value: e.target.value })} />
              <label style={{ color: C.sub, fontSize: 12, display: "flex", alignItems: "center", gap: 6, whiteSpace: "nowrap" }}>
                <input type="checkbox" checked={field.secret} onChange={(e) => updateExtra(index, { secret: e.target.checked })} />
                隐藏
              </label>
              <Button onClick={() => setA((prev) => ({ ...prev, extra_fields: prev.extra_fields.filter((_, i) => i !== index) }))}>删除</Button>
            </div>
          ))}
        </Section>
        </div>
      </div>

      {/* 字段多了以后底部按钮会被滚到看不见，钉在底下 */}
      <div style={{ flexShrink: 0, borderTop: `1px solid ${C.border}`, background: C.white, padding: "12px 0" }}>
        <div style={{ width: 680, margin: "0 auto", display: "flex", alignItems: "center", gap: 10, justifyContent: "flex-end" }}>
          <ErrorLine text={err} />
          <div style={{ flexGrow: 1 }} />
          <Button onClick={onCancel}>取消</Button>
          <Button kind="primary" onClick={submit}>
            保存
          </Button>
        </div>
      </div>
    </div>
  );
}

/** 一组相关字段一张卡，标题在卡外面，扫一眼就知道这段是干嘛的 */
function Section({ title, desc, action, children }: { title: string; desc?: string; action?: React.ReactNode; children: React.ReactNode }) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
      <div style={{ display: "flex", alignItems: "baseline", justifyContent: "space-between", gap: 12, padding: "0 2px" }}>
        <div style={{ display: "flex", alignItems: "baseline", gap: 8, minWidth: 0 }}>
          <span style={{ fontSize: 14, fontWeight: 500, color: C.text }}>{title}</span>
          {desc && <span style={{ fontSize: 12, color: C.muted }}>{desc}</span>}
        </div>
        {action}
      </div>
      <Card style={{ display: "flex", flexDirection: "column", gap: 16, padding: 20 }}>{children}</Card>
    </div>
  );
}
