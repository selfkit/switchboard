/**
 * 打包后把 dist 里的 JS 再过一道混淆。
 *
 * 先说清楚它**不**做什么，免得误以为有了它就安全：
 * - 这不是加密。字节码照样能跑，照样能被还原读懂，只是费劲。
 * - 真正的秘密（账号密码、TOTP 密钥）在 SQLCipher 库里，靠主密码保护，跟这里无关。
 * - 业务逻辑在 Rust 侧，前端只是界面 + invoke 调用，本来就没什么可偷的。
 *
 * 它实际解决的是：`grep` 二进制能捞出界面文案这件事。开了 stringArray +
 * unicodeEscapeSequence 之后，字符串进了 base64 数组、中文变 \uXXXX，搜不到了。
 *
 * 配置刻意保守：controlFlowFlattening / deadCodeInjection / selfDefending 全关。
 * 那几项对 React 这种大量依赖字符串比较和特性探测的库有实打实的运行期风险，
 * 换来的只是"更难读一点"，不值。
 */
import { readdirSync, readFileSync, writeFileSync, statSync } from "node:fs";
import { join } from "node:path";
// 这个包是 CommonJS，只能走默认导出
import JavaScriptObfuscator from "javascript-obfuscator";

const dir = "dist/assets";
const files = readdirSync(dir).filter((f) => f.endsWith(".js"));
if (files.length === 0) {
  console.error("dist/assets 下没有 JS，先跑 vite build");
  process.exit(1);
}

for (const f of files) {
  const p = join(dir, f);
  const before = statSync(p).size;
  const out = JavaScriptObfuscator.obfuscate(readFileSync(p, "utf8"), {
    compact: true,
    stringArray: true,
    stringArrayThreshold: 0.8,
    stringArrayEncoding: ["base64"],
    unicodeEscapeSequence: true,
    identifierNamesGenerator: "mangled",
    numbersToExpressions: true,
    simplify: true,
    // 下面这些有运行期风险，明确关掉
    controlFlowFlattening: false,
    deadCodeInjection: false,
    selfDefending: false,
    debugProtection: false,
    splitStrings: false,
  }).getObfuscatedCode();
  writeFileSync(p, out);
  const kb = (n) => (n / 1024).toFixed(0);
  console.log(`混淆 ${f}: ${kb(before)}KB → ${kb(out.length)}KB`);
}
