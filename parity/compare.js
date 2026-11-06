// Compares two vw-dump-shaped JSON files. Node and link order is ignored (the native
// app lays the graph out itself); everything else must match. mtime may differ by
// floating-point noise only.
const fs = require('fs');
const [aPath, bPath, mode] = process.argv.slice(2);
const a = JSON.parse(fs.readFileSync(aPath, 'utf8'));
const b = JSON.parse(fs.readFileSync(bPath, 'utf8'));
const problems = [];
const say = m => { if (problems.length < 25) problems.push(m); else if (problems.length === 25) problems.push('…more'); };

if (mode === 'excerpts') {
  const keys = new Set([...Object.keys(a), ...Object.keys(b)]);
  for (const k of keys) if (a[k] !== b[k]) say(`excerpt ${k}\n   js:   ${JSON.stringify(a[k])}\n   rust: ${JSON.stringify(b[k])}`);
  console.log(problems.length ? `MISMATCH (${keys.size} excerpts)\n` + problems.join('\n') : `match: ${keys.size} excerpts identical`);
  process.exit(problems.length ? 1 : 0);
}

const norm = n => { const { mtime, ...rest } = n; return { rest: JSON.stringify(Object.entries(rest).sort()), mtime }; };
const byId = g => new Map(g.nodes.map(n => [n.id, n]));
const an = byId(a), bn = byId(b);
if (an.size !== a.nodes.length) say(`js has duplicate node ids`);
if (bn.size !== b.nodes.length) say(`rust has duplicate node ids`);
for (const [id, n] of an) {
  const m = bn.get(id);
  if (!m) { say(`node only in js: ${JSON.stringify(n)}`); continue; }
  const x = norm(n), y = norm(m);
  if (x.rest !== y.rest) say(`node differs ${id}\n   js:   ${JSON.stringify(n)}\n   rust: ${JSON.stringify(m)}`);
  if ((x.mtime === undefined) !== (y.mtime === undefined) || Math.abs((x.mtime || 0) - (y.mtime || 0)) > 0.01)
    say(`mtime differs ${id}: ${x.mtime} vs ${y.mtime}`);
}
for (const [id, m] of bn) if (!an.has(id)) say(`node only in rust: ${JSON.stringify(m)}`);
const linkSet = g => new Set(g.links.map(l => l.source + '\u0000' + l.target));
const al = linkSet(a), bl = linkSet(b);
if (al.size !== a.links.length) say('js has duplicate links');
if (bl.size !== b.links.length) say('rust has duplicate links');
for (const l of al) if (!bl.has(l)) say(`link only in js: ${l.replace('\u0000', ' -> ')}`);
for (const l of bl) if (!al.has(l)) say(`link only in rust: ${l.replace('\u0000', ' -> ')}`);
if (a.tagged !== b.tagged) say(`tagged ${a.tagged} vs ${b.tagged}`);
if (JSON.stringify(Object.entries(a.discoveredTags).sort()) !== JSON.stringify(Object.entries(b.discoveredTags).sort())) say('discoveredTags differ');
const summary = `${a.nodes.length} nodes (${a.nodes.filter(n => n.ghost).length} ghosts), ${a.links.length} links, ${a.tagged} tagged`;
console.log(problems.length ? `MISMATCH vs js ${summary}\n` + problems.join('\n') : `match: ${summary}`);
process.exit(problems.length ? 1 : 0);
