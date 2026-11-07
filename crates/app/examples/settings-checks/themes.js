(async () => {
  const L = m => window.ipc.postMessage(m);
  const sleep = ms => new Promise(r => setTimeout(r, ms));
  const disk = async () => (await fetch('vw://app/api/config')).json();
  L('hasFocus=' + document.hasFocus());
  const tiles = () => Array.from(document.querySelectorAll('.theme-tile')).map(b => b.dataset.theme + (b.getAttribute('aria-pressed') === 'true' ? '*' : ''));
  L('tiles=' + tiles().join(','));
  L('rotation=' + Array.from(document.querySelectorAll('#rotateThemes input')).map(i => i.value).join(','));
  for (const t of await (await fetch('vw://app/api/themes')).json()) if (t.preview) { const r = await fetch('vw://app' + t.preview); L('preview ' + t.id + ' ' + r.status + ' ' + (await r.blob()).size); }
  L('motion options=' + Array.from(document.getElementById('f-motionMode').options).map(o => o.textContent).join('|') + ' value=' + document.getElementById('f-motionMode').value);
  L('webonly badges=' + document.querySelectorAll('.row .webonly').length);
  // a web theme in the config (imported web config)
  await fetch('vw://app/api/config', {method:'POST', headers:{'Content-Type':'application/json'}, body: JSON.stringify({theme:'bauhaus', rotate:{themes:['metro','night-sky'], everyMinutes:30}})});
  await window.vwConfigChanged(); await sleep(200);
  L('after web theme tiles=' + tiles().join(',') + ' rotation=' + Array.from(document.querySelectorAll('#rotateThemes input')).map(i => i.value + (i.checked ? '+' : '')).join(','));
  document.querySelector('.theme-tile[data-theme="aurora-lite"]').click(); await sleep(300);
  L('clicked aurora-lite -> disk=' + (await disk()).theme + ' tiles=' + tiles().join(','));
  // keyboard: Tab from the custom name field should go to the Save button
  const inp = document.getElementById('customName'); inp.focus(); L('focus customName -> active=' + document.activeElement.id);
  document.getElementById('resetDefaults').click(); document.getElementById('resetYes').click(); await sleep(400);
  L('reset motion=' + (await disk()).motionMode + ' theme=' + (await disk()).theme);
  document.querySelector('.theme-tile[data-theme="ink-paper"]').click(); await sleep(300);
  window.scrollTo(0, 0);
  L('DONE');
})().catch(e => window.ipc.postMessage('SCRIPT ERROR ' + e.message + ' ' + e.stack));
