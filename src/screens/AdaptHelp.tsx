import { useState } from "react";
import * as api from "../api";
import { Button, C, ErrorLine, copyText, inputStyle } from "../ui";

/** 在登录页控制台跑这段，把所有输入框的结构打出来，喂给 AI */
const PROBE_SNIPPET = `copy(JSON.stringify([...document.querySelectorAll('input,textarea')].map(e=>({
  tag:e.tagName, type:e.type, id:e.id, name:e.name,
  placeholder:e.placeholder, ariaLabel:e.getAttribute('aria-label'),
  cls:e.className, visible:e.getClientRects().length>0
})),null,2))`;

const PROMPT = `我在做一个账号管理器，它会往登录页里自动填账号密码，需要一份「平台适配配置」。

下面是某个登录页里所有输入框的结构（我在该页面控制台里抓的）：

<<<把上一步复制到的 JSON 粘贴到这里>>>

请帮我挑出【账号输入框】和【密码输入框】，并只输出一段 JSON，不要任何解释文字，格式严格如下：

[
  {
    "platform": "平台名称",
    "username_selector": "账号输入框的 CSS 选择器",
    "password_selector": "密码输入框的 CSS 选择器",
    "trigger_event": "input+change"
  }
]

要求：
1. 选择器用 document.querySelector 能直接命中，优先用 id，其次 name，都没有再用 placeholder 或 type 等属性；不要用会随构建变化的哈希类名。
2. 页面上如果同时存在「扫码登录」和「账号密码登录」两套表单，选择器要能同时命中两套没关系，我的程序只会填其中可见的那个。
3. trigger_event 填 "input+change"；如果这个平台只能扫码、根本没有账号密码框，就填 "skip" 并把两个 selector 留空字符串。
4. platform 用我告诉你的平台名：<<<在这里写平台名，要和 App 里那一列完全一致>>>`;

function CopyBlock({ label, text, note }: { label: string; text: string; note?: string }) {
  const [copied, setCopied] = useState<boolean | null>(null);
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
      <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
        <span style={{ fontSize: 13, color: C.text, fontWeight: 500 }}>{label}</span>
        <Button
          onClick={async () => {
            setCopied(await copyText(text));
            setTimeout(() => setCopied(null), 1600);
          }}
          style={{ padding: "4px 10px", fontSize: 12 }}
        >
          {copied === true ? "已复制" : copied === false ? "复制失败" : "复制"}
        </Button>
      </div>
      {note && <div style={{ fontSize: 12, color: C.muted, lineHeight: 1.6 }}>{note}</div>}
      <pre
        style={{
          margin: 0,
          background: C.bg,
          borderRadius: 8,
          padding: "12px 14px",
          fontSize: 11,
          lineHeight: 1.6,
          color: C.sub,
          whiteSpace: "pre-wrap",
          wordBreak: "break-word",
          maxHeight: 200,
          overflowY: "auto",
          fontFamily: "ui-monospace, monospace",
        }}
      >
        {text}
      </pre>
    </div>
  );
}

/**
 * 「在线适配」：新平台不用等我重新打包。
 * 自己去登录页抓一次输入框结构 → 丢给 AI → 把它吐的 JSON 贴回来导入。
 */
export default function AdaptHelp({ onClose, onImported }: { onClose: () => void; onImported: () => void }) {
  const [json, setJson] = useState("");
  const [err, setErr] = useState("");
  const [msg, setMsg] = useState("");

  async function doImport() {
    setErr("");
    setMsg("");
    try {
      const n = await api.importPlatformConfigs(json);
      setMsg(`已导入 ${n} 条配置，立即生效`);
      setJson("");
      onImported();
    } catch (e) {
      setErr(String(e));
    }
  }

  async function doExport() {
    setErr("");
    setMsg("");
    try {
      const text = await api.exportPlatformConfigs();
      if (!(await copyText(text))) throw new Error("复制失败，请检查剪贴板权限");
      setMsg("当前全部配置已复制到剪贴板");
    } catch (e) {
      setErr(String(e));
    }
  }

  const step = (n: number, title: string, body: React.ReactNode) => (
    <div style={{ display: "flex", gap: 12 }}>
      <div
        style={{
          width: 22,
          height: 22,
          borderRadius: 999,
          background: C.brandSoft,
          color: C.brand,
          fontSize: 12,
          fontWeight: 600,
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          flexShrink: 0,
        }}
      >
        {n}
      </div>
      <div style={{ display: "flex", flexDirection: "column", gap: 8, flexGrow: 1, minWidth: 0 }}>
        <div style={{ fontSize: 13, color: C.text, fontWeight: 500 }}>{title}</div>
        {body}
      </div>
    </div>
  );

  return (
    <div
      onClick={onClose}
      style={{ position: "fixed", inset: 0, background: "rgba(0,0,0,0.35)", display: "flex", alignItems: "flex-start", justifyContent: "center", padding: "48px 0", overflowY: "auto" }}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        style={{ width: 680, background: C.surface, borderRadius: 14, padding: 28, display: "flex", flexDirection: "column", gap: 22 }}
      >
        <div>
          <div style={{ fontSize: 18, color: C.text }}>新平台怎么适配（不用重新打包）</div>
          <div style={{ fontSize: 13, color: C.sub, marginTop: 6, lineHeight: 1.7 }}>
            自动填充要知道"账号框和密码框是哪两个"。遇到没适配过的网站，有两条路：
            <br />
            <b>最省事</b>：到「会话」页打开该账号，点左边的「指认输入框」，再到右边页面点一下那两个框，自动写回。
            <br />
            <b>这里这条</b>：适合页面结构复杂、手动点不准的情况——抓一次结构丢给 AI，把它给的 JSON 贴回来。
          </div>
        </div>

        {step(
          1,
          "在那个登录页的浏览器控制台里，跑这段",
          <CopyBlock
            label="抓取脚本"
            text={PROBE_SNIPPET}
            note="用你平时的 Chrome 打开该登录页 → F12 → Console → 粘贴回车。它会把页面上所有输入框的结构直接复制到你的剪贴板。"
          />,
        )}

        {step(
          2,
          "把这段提示词发给 ChatGPT / Claude",
          <CopyBlock label="提示词" text={PROMPT} note="粘贴后，把第 1 步抓到的 JSON 和平台名填进提示词里的两处尖括号占位。" />,
        )}

        {step(
          3,
          "把 AI 回的 JSON 贴到这里导入",
          <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
            <textarea
              value={json}
              onChange={(e) => setJson(e.target.value)}
              rows={7}
              placeholder={'[\n  {\n    "platform": "某某云",\n    "username_selector": "#loginName",\n    "password_selector": "#loginPassword",\n    "trigger_event": "input+change"\n  }\n]'}
              style={{ ...inputStyle, fontFamily: "ui-monospace, monospace", fontSize: 12, resize: "vertical" }}
            />
            <div style={{ display: "flex", gap: 10, alignItems: "center" }}>
              <Button kind="primary" onClick={doImport} disabled={!json.trim()}>
                导入并生效
              </Button>
              <Button onClick={doExport}>复制当前全部配置</Button>
            </div>
            <div style={{ fontSize: 12, color: C.muted, lineHeight: 1.7 }}>
              platform 和账号里填的平台名<b>必须完全一致</b>，否则匹配不上（列表里没有的平台名会新增一行）。
              导入会覆盖同名平台的旧配置，下次打开该平台的账号立刻生效，不用重启。
            </div>
          </div>,
        )}

        <ErrorLine text={err} />
        {msg && <div style={{ fontSize: 13, color: C.brand }}>{msg}</div>}

        <div style={{ display: "flex", justifyContent: "flex-end" }}>
          <Button onClick={onClose}>关闭</Button>
        </div>
      </div>
    </div>
  );
}
