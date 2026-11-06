// Generates a synthetic vault: N notes in nested folders, ~3.5 links and 1-2 tags each.
// Usage: node make-big.js <dir> [notes=35000]
const fs = require('fs'), path = require('path');
const [root, nArg] = process.argv.slice(2);
const n = parseInt(nArg || '35000', 10);
fs.rmSync(root, { recursive: true, force: true });
let seed = 42; const rnd = () => (seed = (seed * 1103515245 + 12345) % 2147483648) / 2147483648;
const tags = Array.from({ length: 40 }, (_, i) => `topic${i}`);
for (let i = 0; i < n; i++) {
  const dir = path.join(root, `area${i % 12}`, `sub${i % 37}`);
  if (i < 12 * 37) fs.mkdirSync(dir, { recursive: true });
  const links = Array.from({ length: 1 + Math.floor(rnd() * 5) }, () => `[[Note ${Math.floor(rnd() ** 2 * n)}]]`);
  const t = [tags[Math.floor(rnd() * 40)], tags[Math.floor(rnd() * 40)]];
  const body = `---\ntags: [${t[0]}]\n---\n# Note ${i}\n${'Lorem ipsum dolor sit amet. '.repeat(1 + (i % 20))}\n${links.join(' ')} #${t[1]}\n`;
  fs.writeFileSync(path.join(dir, `Note ${i}.md`), body);
}
console.log(`${n} notes in ${root}`);
