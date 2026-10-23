const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const source = require('./origin-policy.cjs');
const marker = 'window.__sbOriginAllowed = allowed;';
assert(source.includes(marker));
const context = { URL }; context.window = context;
// Expose the actual internal resolver only inside this test context.
vm.runInNewContext(source.replace(marker, marker + '\nwindow.testRegisteredDomain = registeredDomain;'), context);
const ascii = host => host === null ? null : new URL('https://' + host).hostname;
const vectors = JSON.parse(fs.readFileSync(path.join(__dirname, 'fixtures/public-suffix-cases.json'), 'utf8'));
for (const [host, expected] of vectors) {
  const actual = host === null ? null : context.testRegisteredDomain(ascii(host));
  assert.equal(actual, ascii(expected), 'PSL upstream vector: ' + host);
}
console.log('PASS upstream public suffix vectors:', vectors.length);

const policy = context.__sbOriginAllowed;
const allowed = [
  ['https://admin.example.com/', 'https://login.example.com/auth'],
  ['https://example.com/', 'https://auth.internal.example.com/'],
  ['https://admin.company.com.cn/', 'https://login.company.com.cn/'],
  ['https://admin.example.co.uk/', 'https://login.example.co.uk/'],
  ['https://admin.example.com:8443/', 'https://login.example.com:9443/'],
  ['https://admin.食狮.公司.cn/', 'https://login.xn--85x722f.xn--55qx5d.cn/'],
  ['https://admin.example.com./', 'https://login.example.com/'],
  ['https://auth.a.github.io/', 'https://login.a.github.io/'],
  ['https://auth.www.ck/', 'https://login.www.ck/'],
  ['https://foo.city.kobe.jp/', 'https://bar.city.kobe.jp/'],
  ['http://localhost:8000/a', 'http://localhost:8000/b'],
  ['http://127.0.0.1:8000/a', 'http://127.0.0.1:8000/b'],
];
const denied = [
  ['https://a.github.io/', 'https://b.github.io/'],
  ['https://a.appspot.com/', 'https://b.appspot.com/'],
  ['https://a.vercel.app/', 'https://b.vercel.app/'],
  ['https://a.co.uk/', 'https://b.co.uk/'],
  ['https://a.example.com/', 'https://a.example.net/'],
  ['https://a.example.com/', 'https://a.example.com.attacker.test/'],
  ['https://foo.test.ck/', 'https://bar.other.ck/'],
  ['https://signin.aliyun.com/', 'http://passport.aliyun.com/'],
  ['http://localhost:8000/', 'http://localhost:9000/'],
  ['http://127.0.0.1:8000/', 'http://127.0.0.1:9000/'],
  ['http://[::1]:8000/', 'http://[::1]:9000/'],
  ['https://github.io/', 'https://a.github.io/'],
  ['file:///a', 'file:///b'],
  ['null', 'https://example.com/'],
];
for (const [a, b] of allowed) assert(policy(a, b), `${a} -> ${b}`);
for (const [a, b] of denied) assert(!policy(a, b), `${a} -> ${b}`);
console.log('PASS same-site login and tenant isolation:', allowed.length + denied.length);
