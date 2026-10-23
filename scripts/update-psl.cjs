// Download separately, then: node scripts/update-psl.cjs /path/to/public_suffix_list.dat [/path/to/test_psl.txt]
// Canonicalize rule hostnames once so Rust and the injected JS use identical offline data.
const fs = require('node:fs');
const path = require('node:path');
const { domainToASCII } = require('node:url');
const source = fs.readFileSync(process.argv[2], 'utf8');
for (const marker of ['// ===BEGIN ICANN DOMAINS===', '// ===BEGIN PRIVATE DOMAINS===']) {
  if (!source.includes(marker)) throw new Error('Missing PSL section: ' + marker);
}
let count = 0;
const normalized = source.split(/\r?\n/).map(line => {
  const rule = line.trim().split(/\s+/)[0];
  if (!rule || rule.startsWith('//')) return line;
  const prefix = rule.startsWith('!') ? '!' : rule.startsWith('*.') ? '*.' : '';
  const host = domainToASCII(rule.slice(prefix.length));
  if (!host || host.startsWith('.') || host.includes('..')) throw new Error('Invalid PSL rule: ' + rule);
  count++;
  return prefix + host;
}).join('\n');
if (count < 5000) throw new Error('Incomplete PSL');
const target = path.join(__dirname, '../src-tauri/resources/public_suffix_list.dat');
fs.mkdirSync(path.dirname(target), { recursive: true });
fs.writeFileSync(target, normalized.replace('// Instructions on pulling', '// Modified by Switchboard: rule hostnames canonicalized to lowercase ASCII/Punycode.\n// Instructions on pulling'));
console.log('Updated offline PSL:', count, 'rules');
if (process.argv[3]) {
  const tests = fs.readFileSync(process.argv[3], 'utf8');
  const vectors = tests.split(/\r?\n/).flatMap(line => {
    const match = line.trim().match(/^checkPublicSuffix\((null|'[^']*'),\s*(null|'[^']*')\);$/);
    return match ? [match.slice(1).map(value => value === 'null' ? null : value.slice(1, -1))] : [];
  });
  if (vectors.length < 70) throw new Error('Incomplete upstream PSL test vectors');
  const fixtures = path.join(__dirname, 'fixtures');
  fs.mkdirSync(fixtures, { recursive: true });
  fs.writeFileSync(path.join(fixtures, 'public-suffix-tests.txt'), tests);
  fs.writeFileSync(path.join(fixtures, 'public-suffix-cases.json'), JSON.stringify(vectors, null, 2) + '\n');
  console.log('Updated upstream PSL test vectors:', vectors.length);
}
