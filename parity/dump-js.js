// Prints what the web version's parser.js produces for a vault, in the same shape as
// `vw-dump`. Usage: node dump-js.js <parser.js> <vault> [--ignore p]... [--no-unresolved] [--excerpts]
const fs = require('fs');
const [parserPath, vault, ...rest] = process.argv.slice(2);
const { scanVaultEntries, materializeGraph, noteExcerpt } = require(parserPath);
const ignore = [];
let showUnresolvedLinks = true, excerpts = false;
for (let i = 0; i < rest.length; i++) {
  if (rest[i] === '--ignore') ignore.push(rest[++i]);
  else if (rest[i] === '--no-unresolved') showUnresolvedLinks = false;
  else if (rest[i] === '--excerpts') excerpts = true;
}
// parser.js logs duplicate-basename notes to stdout; keep stdout for the JSON.
console.log = (...a) => process.stderr.write(a.join(' ') + '\n');
scanVaultEntries(vault, ignore).then(entries => {
  if (excerpts) {
    const out = {};
    for (const p of [...entries.keys()].sort()) out[p] = noteExcerpt(fs.readFileSync(p, 'utf8'));
    process.stdout.write(JSON.stringify(out) + '\n');
    return;
  }
  const r = materializeGraph(entries, vault, { showUnresolvedLinks });
  process.stdout.write(JSON.stringify({ nodes: r.nodes, links: r.links, tagged: r.tagged, discoveredTags: r.discoveredTags }) + '\n');
});
