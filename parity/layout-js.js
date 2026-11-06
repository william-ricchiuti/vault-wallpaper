// Runs the web version's layout (layout-core.js + d3) on a vault, with the worker's
// settle rule, and writes positions for draw_layout. Usage:
//   node layout-js.js <web checkout> <vault> <out.json> [force|rings|timeline|galaxy]
const fs = require('fs'), path = require('path');
const [web, vault, out, kind = 'force'] = process.argv.slice(2);
const { scanVaultEntries, materializeGraph } = require(path.join(web, 'parser.js'));
const d3 = require(path.join(web, 'vendor/d3.min.js'));
const L = require(path.join(web, 'layout-core.js'));
console.log = (...a) => process.stderr.write(a.join(' ') + '\n');
(async () => {
  const g = materializeGraph(await scanVaultEntries(vault, []), vault, {});
  const deg = new Map();
  for (const l of g.links) { deg.set(l.source, (deg.get(l.source) || 0) + 1); deg.set(l.target, (deg.get(l.target) || 0) + 1); }
  const W = 1920, H = 1080, spread = Math.min(W, H) * 0.4;
  const nodes = g.nodes.map(n => ({ id: n.id, tag: n.tag || null, ghost: !!n.ghost, mtime: n.mtime || 0,
    r: n.ghost ? 1.5 + Math.sqrt(deg.get(n.id) || 0) * 0.8 : 2.2 + Math.sqrt(deg.get(n.id) || 0) * 1.45,
    x: W / 2 + (Math.random() - 0.5) * spread, y: H / 2 + (Math.random() - 0.5) * spread, vx: 0, vy: 0 }));
  const links = g.links.map(l => ({ source: l.source, target: l.target }));
  const t0 = performance.now();
  const sim = L.createSimulation(d3, nodes, links, { layout: kind, width: W, height: H, centerX: W / 2, centerY: H / 2, clusterByTag: true, alpha: 1, now: Date.now() });
  let ticks = 0, idle = 0;
  while (idle < 30 && ticks < 600) { sim.tick(); ticks++; idle = sim.alpha() < 0.02 ? idle + 1 : 0; }
  const ms = performance.now() - t0;
  process.stderr.write(`js ${kind}: ${nodes.length} nodes, ${links.length} links: ${ticks} ticks, ${ms.toFixed(0)} ms (${(ms / ticks).toFixed(2)} ms/tick)\n`);
  const map = {}; for (const n of nodes) map[n.id] = [n.x - W / 2, n.y - H / 2];
  fs.writeFileSync(out, JSON.stringify(map));
})();
