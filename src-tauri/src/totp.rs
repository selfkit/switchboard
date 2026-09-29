//! 账号级可选的本地动态验证码。跟登录流程解耦，只有账号填了 totp_secret 才用得上。

use serde::Serialize;
use std::collections::HashMap;
use totp_rs::{Algorithm, Secret, TOTP};

#[derive(Serialize)]
pub struct TotpCode {
    /// 6 位验证码
    pub code: String,
    /// 距离下次刷新还剩几秒
    pub remaining: u64,
}

pub fn generate(secret: &str) -> Result<TotpCode, String> {
    // 末尾的 = 是 Base32 补位，有的平台带、有的不带，解码器不认它
    let cleaned: String = secret.chars().filter(|c| !c.is_whitespace()).collect::<String>().trim_end_matches('=').to_string();
    if cleaned.is_empty() {
        return Err("这个账号没有绑定 TOTP 密钥".into());
    }
    let bytes = Secret::Encoded(cleaned.to_uppercase())
        .to_bytes()
        .map_err(|_| "TOTP 密钥不是合法的 Base32".to_string())?;
    // 不用 TOTP::new：它按 RFC 建议卡死"至少 128 位"，而 GitHub 等大量平台发的就是 16 个字符（80 位）的密钥，
    // 卡掉的话这些账号手机能出码、这里却报错。长度够不够是平台的事，不归我们拦
    let totp = TOTP::new_unchecked(Algorithm::SHA1, 6, 1, 30, bytes);
    Ok(TotpCode {
        code: totp.generate_current().map_err(|e| e.to_string())?,
        remaining: totp.ttl().map_err(|e| e.to_string())?,
    })
}

/// 从 otpauth:// 链接里取密钥。平台绑定两步验证时显示的二维码里装的就是这种链接。
/// 一张图里可能有好几个码（识别结果一行一个），挑第一个 TOTP 的。
///
/// 只收标准的 SHA1 · 6 位 · 30 秒：`generate` 只会算这一种，参数不同还硬收的话，
/// 算出来的码永远对不上手机，比当场说"不支持"难查得多。
pub fn secret_from_uri(text: &str) -> Result<String, String> {
    let Some(uri) = text.lines().map(str::trim).find(|l| l.starts_with("otpauth://totp/")) else {
        return Err(if text.contains("otpauth-migration://") {
            "这是 Google 身份验证器「导出账号」的二维码，暂不支持；请用平台绑定两步验证时显示的二维码".into()
        } else if text.contains("otpauth://hotp") {
            "这是按次数变化的 HOTP 验证码，只支持每 30 秒一变的 TOTP".into()
        } else if text.trim().is_empty() {
            "没认出二维码：截图里要有完整的二维码，别太小或太糊".into()
        } else {
            format!("认出来的不是验证器二维码：{}", text.trim().chars().take(60).collect::<String>())
        });
    };
    let url: tauri::Url = uri.parse().map_err(|_| "二维码里的链接格式不对".to_string())?;
    let q: HashMap<String, String> = url.query_pairs().map(|(k, v)| (k.to_ascii_lowercase(), v.into_owned())).collect();
    let secret: String = q.get("secret").map(|s| s.split_whitespace().collect()).unwrap_or_default();
    let secret = secret.trim_end_matches('=').to_uppercase();
    if secret.is_empty() {
        return Err("二维码里没有密钥（secret）".into());
    }
    let algo = q.get("algorithm").map(|s| s.to_ascii_uppercase()).unwrap_or_else(|| "SHA1".into());
    let digits = q.get("digits").map(String::as_str).unwrap_or("6");
    let period = q.get("period").map(String::as_str).unwrap_or("30");
    if algo != "SHA1" || digits != "6" || period != "30" {
        return Err(format!("这个验证码是 {algo} · {digits} 位 · {period} 秒，目前只支持最常见的 SHA1 · 6 位 · 30 秒"));
    }
    generate(&secret)?; // 不是合法 Base32 就当场说
    Ok(secret)
}

/// 用系统自带的 CoreImage 认二维码（osascript 跑一段 JXA），不为这一个功能引一个图像解码库。
/// `path` 为空就读剪贴板里的图片——⌘⌃⇧4 截图默认进剪贴板，PNG / TIFF 都认。
const QR_JS: &str = r#"ObjC.import('CoreImage'); ObjC.import('AppKit');
function run(argv) {
  var img;
  if (argv.length) img = $.CIImage.imageWithContentsOfURL($.NSURL.fileURLWithPath(argv[0]));
  else {
    var pb = $.NSPasteboard.generalPasteboard;
    var data = pb.dataForType($.NSPasteboardTypePNG);
    if (data.isNil()) data = pb.dataForType($.NSPasteboardTypeTIFF);
    if (data.isNil()) return '';
    img = $.CIImage.imageWithData(data);
  }
  if (img.isNil()) return '';
  var det = $.CIDetector.detectorOfTypeContextOptions($.CIDetectorTypeQRCode, $(),
    $.NSDictionary.dictionaryWithObjectForKey($.CIDetectorAccuracyHigh, $.CIDetectorAccuracy));
  var fs = det.featuresInImage(img), out = [];
  for (var i = 0; i < fs.count; i++) out.push(ObjC.unwrap(fs.objectAtIndex(i).messageString));
  return out.join('\n');
}"#;

/// 认出图片里的二维码原文，多个码一行一个；没有码返回空串
pub fn read_qr(path: Option<&str>) -> Result<String, String> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (path, QR_JS);
        return Err("识别二维码目前只支持 macOS，请手动粘贴密钥".into());
    }
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("osascript")
            .args(["-l", "JavaScript", "-e", QR_JS])
            .args(path)
            .output()
            .map_err(|e| format!("识别二维码失败：{e}"))?;
        if !out.status.success() {
            return Err(format!("识别二维码失败：{}", String::from_utf8_lossy(&out.stderr).trim()));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc_test_vector() {
        // RFC 6238 的 "12345678901234567890"，Base32 = GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ
        let bytes = Secret::Encoded("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ".into()).to_bytes().unwrap();
        let totp = TOTP::new(Algorithm::SHA1, 8, 1, 30, bytes).unwrap();
        assert_eq!(totp.generate(59), "94287082");
        assert_eq!(totp.generate(1111111109), "07081804");
    }

    #[test]
    fn accepts_spaced_lowercase_secret_and_rejects_junk() {
        let a = generate("gezd gnbv gy3t qojq gezd gnbv gy3t qojq").unwrap();
        assert_eq!(a.code.len(), 6);
        assert!(a.remaining <= 30);
        assert!(generate("").is_err());
        assert!(generate("这不是 base32 ~!@#").is_err());
        // 16 个字符（80 位）的密钥很常见，带补位的 = 也得认
        assert!(generate("JBSWY3DPEHPK3PXP").is_ok(), "短密钥被拦了");
        assert_eq!(generate("JBSWY3DPEHPK3PXP====").unwrap().code, generate("JBSWY3DPEHPK3PXP").unwrap().code);
    }

    #[test]
    fn secret_comes_out_of_otpauth_links() {
        let ok = |t: &str| secret_from_uri(t).unwrap();
        assert_eq!(ok("otpauth://totp/Aliyun:tan@x.com?secret=jbswy3dpehpk3pxp&issuer=Aliyun"), "JBSWY3DPEHPK3PXP");
        // 显式写了默认参数照收；一张图多个码时挑 TOTP 那个
        assert_eq!(ok("https://x.com\notpauth://totp/a?secret=JBSWY3DPEHPK3PXP&algorithm=sha1&digits=6&period=30"), "JBSWY3DPEHPK3PXP");
        let err = |t: &str| secret_from_uri(t).unwrap_err();
        assert!(err("otpauth://totp/a?secret=JBSWY3DPEHPK3PXP&digits=8").contains("8 位"), "算不对的参数要当场拒");
        assert!(err("otpauth://totp/a?secret=JBSWY3DPEHPK3PXP&algorithm=SHA256").contains("SHA256"));
        assert!(err("otpauth-migration://offline?data=xx").contains("导出账号"));
        assert!(err("otpauth://hotp/a?secret=JBSWY3DPEHPK3PXP&counter=1").contains("HOTP"));
        assert!(err("otpauth://totp/a?issuer=x").contains("没有密钥"));
        assert!(err("").contains("没认出"));
        assert!(err("https://example.com").contains("不是验证器"));
    }

    /// 真跑一遍 osascript：先用 CoreImage 生成一张二维码图，再让 read_qr 认回来。
    /// 这段 JXA 靠的是系统框架，macOS 升级改了什么，这里会先红
    #[cfg(target_os = "macos")]
    #[test]
    fn reads_a_real_qr_image() {
        let path = std::env::temp_dir().join(format!("sb-qr-{}.png", std::process::id()));
        let gen = r#"ObjC.import('CoreImage'); ObjC.import('AppKit');
function run(a) {
  var f = $.CIFilter.filterWithName('CIQRCodeGenerator');
  f.setValueForKey($(a[0]).dataUsingEncoding($.NSUTF8StringEncoding), 'inputMessage');
  var rep = $.NSBitmapImageRep.alloc.initWithCIImage(f.outputImage.imageByApplyingTransform($.CGAffineTransformMakeScale(10, 10)));
  rep.representationUsingTypeProperties($.NSBitmapImageFileTypePNG, $()).writeToFileAtomically(a[1], true);
}"#;
        let uri = "otpauth://totp/Demo:me?secret=JBSWY3DPEHPK3PXP&issuer=Demo";
        let ok = std::process::Command::new("osascript")
            .args(["-l", "JavaScript", "-e", gen, uri])
            .arg(&path)
            .status()
            .unwrap()
            .success();
        assert!(ok, "生成测试二维码失败");
        let text = read_qr(path.to_str()).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(secret_from_uri(&text).unwrap(), "JBSWY3DPEHPK3PXP");
    }
}

