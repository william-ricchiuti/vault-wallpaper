#!/usr/bin/env node
// Config parity: feeds configs through parser.js's sanitizePersistedConfig and
// sanitizeConfigPatch and checks vw_core::config agrees.
//
//   node parity/config-js.js <web checkout>
//
// Cases: every presets.json entry (as a patch, and as a whole file), config.example.json,
// and deliberately bad values. Patches must match exactly (accept/reject and the cleaned
// values). For whole files the Rust side is forgiving on purpose: it drops an invalid key
// instead of rejecting the file. So a file must (a) be flagged clean in Rust exactly when
// parser.js accepts it, and (b) come out equal to what parser.js gives for the file with
// the invalid keys removed. The four web-only keys (port, refreshMs, fastHash, renderer)
// are compared for accept/reject only; Rust never outputs them.
const fs = require('fs');
const os = require('os');
const path = require('path');
const { execFileSync } = require('child_process');

const web = process.argv[2];
if (!web) { console.error('usage: config-js.js <obsidian-live-wallpaper checkout>'); process.exit(2); }
const P = require(path.resolve(web, 'parser.js'));
const presets = JSON.parse(fs.readFileSync(path.join(web, 'presets.json'), 'utf8'));
const example = JSON.parse(fs.readFileSync(path.join(web, 'config.example.json'), 'utf8'));
const root = path.dirname(__dirname);

const work = fs.mkdtempSync(path.join(os.tmpdir(), 'vw-config-'));
const VAULT = path.join(work, 'vault'); // must exist: parser.js checks
fs.mkdirSync(VAULT);
const DROPPED = ['port', 'refreshMs', 'fastHash', 'renderer'];

// ---- cases ---------------------------------------------------------------------------
const cases = [];
const file = (name, input) => cases.push({ name: `file: ${name}`, kind: 'file', input });
const patch = (name, input) => cases.push({ name: `patch: ${name}`, kind: 'patch', input });
const both = (name, obj) => { patch(name, obj); file(name, { vaultPath: VAULT, ...obj }); };

for (const p of presets) {
  patch(`preset ${p.name}`, p.config);
  file(`preset ${p.name}`, { vaultPath: VAULT, ...p.config });
}
file('config.example.json', { ...example, vaultPath: VAULT });
file('defaults', { vaultPath: VAULT, ...P.DEFAULTS });
file('minimal', { vaultPath: VAULT });
patch('empty', {});
for (const t of P.THEME_LIST) both(`theme ${t.id}`, { theme: t.id });

const bad = {
  accent: ['#fff', '7c5cff', '#7c5cffaa', '#7C5CFF', '#7c5cf\n', 123, null, ''],
  lightBackground: ['#ECE6D6', 'ece6d6'],
  linkOpacity: [-0.1, 1.1, '0.5', true, 0, 1, null],
  particleSpeed: [0.09, 0.1, 3, 3.01],
  hubLabelCount: [0, 51, 5.5, 5.0, '5', 1e1],
  maxRenderedNodes: [99, 100000, 100001, 1e3],
  motionMode: ['fast', 'Balanced', 'still', ''],
  edgeStyle: ['none', 'dashed'],
  nodeGlow: [1, 'true', null, false],
  ignorePaths: ['x', [''], ['  '], [1], ['a', 'b'], [], null, {}],
  tagColors: [[], { '': '#ffffff' }, { ' ': '#ffffff' }, { a: 'red' }, { a: '#ffffff', 'b/c': '#000000' }, null, {}],
  labelStyle: [[], { mode: 'x' }, { mode: 'glow', backgroundAlpha: 0.95 }, { glowColor: 'node' }, { glowColor: '#12345g' },
    { glowColor: '#12345F' }, { fontStyle: 'mono', extra: 1 }, { chromaticSplit: 'yes' }, { mode: null }, null, 'badge',
    { mode: 'inherit', backgroundAlpha: 0.9, chromaticSplit: true, fontStyle: 'serif' }],
  safeArea: [{ top: 10 }, { top: -1 }, { top: 1001 }, { top: 1.5 }, { middle: 1 }, [], { top: null }, {},
    { top: 1, right: 2, bottom: 3, left: 4 }, { top: 2.0 }],
  rotate: [{ themes: ['riso', 'aurora'], everyMinutes: 5 }, { themes: ['nope'] }, { themes: 'riso' }, { everyMinutes: 0 },
    { everyMinutes: 1441 }, { everyMinutes: 1440 }, { extra: 1 }, { themes: Array(33).fill('default') },
    { themes: Array(32).fill('wash') }, {}, { themes: null }, [], { everyMinutes: 30.0 }],
  focusNote: ['  hi  ', 'x'.repeat(301), ' '.repeat(10) + 'x'.repeat(290), 1, '😀'.repeat(150), '😀'.repeat(151),
    '﻿x\u0085', '　a ', null],
  theme: ['nope', 'Default', ''],
  layout: ['rings', 'grid'],
  framing: ['none', 'top'],
  palette: ['viridis', 'rainbow'],
  maxFps: [0, 60, 61, -1],
  idleFps: [30, 31],
  zoom: [0.39, 0.4, 3, 3.1],
  offsetX: [-1, 1, 1.01, -0.0],
  focusHops: [0, 4, 5],
  recencyDays: [0, 365, 366],
  depthParallaxLayers: [1, 2, 4, 5],
  refreshMs: [999, 1000, 60000, 60001, '5000'],
  fastHash: ['y', true],
  renderer: ['metal', 'webgl']
};
for (const [key, values] of Object.entries(bad)) {
  values.forEach((v, i) => both(`${key} #${i} ${JSON.stringify(v)}`, { [key]: v }));
}
for (const v of [0, 70000, '3000', 3000, 1.5, null]) file(`port ${JSON.stringify(v)}`, { vaultPath: VAULT, port: v });
for (const v of ['', '   ', 5, null]) file(`vaultPath ${JSON.stringify(v)}`, { vaultPath: v, zoom: 2 });
file('no vaultPath', { zoom: 2 });
file('unknown key', { vaultPath: VAULT, mystery: 1, zoom: 2 });
patch('unknown key', { mystery: 1 });
patch('vaultPath', { vaultPath: VAULT });
patch('port', { port: 3000 });
file('mixed good and bad', { vaultPath: VAULT, accent: 'red', zoom: 2, safeArea: { top: 5 }, rotate: { themes: ['x'] },
  grain: false, port: 0, tagColors: { work: '#ff0000' } });
for (const v of [[], 'x', null, 5]) { file(`root ${JSON.stringify(v)}`, v); patch(`root ${JSON.stringify(v)}`, v); }

// ---- what parser.js says -------------------------------------------------------------
const attempt = fn => { try { return { ok: true, value: fn() }; } catch (e) { return { ok: false, error: e.message }; } };
const strip = obj => { const o = { ...obj }; for (const k of DROPPED) delete o[k]; return o; };
const isObj = v => v !== null && typeof v === 'object' && !Array.isArray(v);

function expectFile(raw) {
  const strict = attempt(() => P.sanitizePersistedConfig(raw));
  if (!isObj(raw)) return { strictOk: false, rootError: true };
  // Per-key verdicts: which keys parser.js would refuse on their own.
  const kept = {};
  const issues = [];
  for (const [k, v] of Object.entries(raw)) {
    let ok;
    if (k === 'vaultPath') ok = typeof v === 'string' && v.trim() !== '';
    else if (k === 'port') ok = attempt(() => P.sanitizePersistedConfig({ vaultPath: VAULT, port: v })).ok;
    else if (P.VALIDATORS[k]) ok = attempt(() => P.sanitizeConfigPatch({ [k]: v })).ok;
    else continue; // unknown: ignored by both
    if (ok) kept[k] = v; else issues.push(k);
  }
  const hadVault = 'vaultPath' in kept;
  const value = strip(P.sanitizePersistedConfig({ vaultPath: VAULT, ...kept }));
  if (!hadVault) delete value.vaultPath;
  return { strictOk: strict.ok, issues: issues.sort(), value };
}

// ---- what Rust says ------------------------------------------------------------------
const casesPath = path.join(work, 'cases.json');
const outPath = path.join(work, 'rust.json');
fs.writeFileSync(casesPath, JSON.stringify(cases));
execFileSync('cargo', ['test', '-q', '-p', 'vw-core', '--lib', '--manifest-path', path.join(root, 'Cargo.toml'),
  'config::tests::parity_dump', '--', '--ignored', '--exact'],
  { env: { ...process.env, VW_CONFIG_CASES: casesPath, VW_CONFIG_OUT: outPath }, stdio: ['ignore', 'ignore', 'inherit'] });
const rust = JSON.parse(fs.readFileSync(outPath, 'utf8'));

// ---- compare -------------------------------------------------------------------------
const canon = v => Array.isArray(v) ? v.map(canon)
  : isObj(v) ? Object.fromEntries(Object.keys(v).sort().map(k => [k, canon(v[k])])) : v;
const same = (a, b) => JSON.stringify(canon(a)) === JSON.stringify(canon(b));

let failures = 0;
const fail = (c, msg) => { failures++; if (failures <= 20) console.log(`  FAIL ${c.name}: ${msg}`); };
cases.forEach((c, i) => {
  const r = rust[i];
  if (c.kind === 'patch') {
    const js = attempt(() => strip(P.sanitizeConfigPatch(c.input)));
    if (js.ok !== r.ok) return fail(c, `parser.js ${js.ok ? 'accepts' : 'rejects (' + js.error + ')'}, Rust ${r.ok ? 'accepts' : 'rejects (' + r.error + ')'}`);
    if (js.ok && !same(js.value, r.value)) fail(c, `values differ\n    js:   ${JSON.stringify(canon(js.value))}\n    rust: ${JSON.stringify(canon(r.value))}`);
    return;
  }
  const e = expectFile(c.input);
  if (e.rootError) { if (r.ok) fail(c, 'Rust accepted a non-object root'); return; }
  if (!r.ok) return fail(c, `Rust rejected the file: ${r.error}`);
  const rustClean = r.issues.length === 0 && r.value.vaultPath !== undefined;
  if (e.strictOk !== rustClean) fail(c, `parser.js ${e.strictOk ? 'accepts' : 'rejects'}, Rust issues ${JSON.stringify(r.issues)}`);
  if (!same(e.issues, [...r.issues].sort())) fail(c, `dropped keys: js ${JSON.stringify(e.issues)}, rust ${JSON.stringify(r.issues)}`);
  if (!same(e.value, r.value)) fail(c, `values differ\n    js:   ${JSON.stringify(canon(e.value))}\n    rust: ${JSON.stringify(canon(r.value))}`);
});

fs.rmSync(work, { recursive: true, force: true });
const files = cases.filter(c => c.kind === 'file').length;
console.log(failures ? `config: ${failures} of ${cases.length} cases differ` :
  `config: ${cases.length} cases identical (${files} files, ${cases.length - files} patches, ${presets.length} presets)`);
process.exit(failures ? 1 : 0);
