// Execute the same policy and the same ASCII PSL snapshot as the native application.
const fs = require('node:fs');
const path = require('node:path');
const root = path.join(__dirname, '../src-tauri');
const rules = fs.readFileSync(path.join(root, 'resources/public_suffix_list.dat'), 'utf8')
  .split(/\r?\n/).map(line => line.trim().split(/\s+/)[0]).filter(line => line && !line.startsWith('//'));
module.exports = fs.readFileSync(path.join(root, 'src/origin-policy.js'), 'utf8')
  .replace('__SB_PUBLIC_SUFFIX_RULES__', JSON.stringify(rules));
