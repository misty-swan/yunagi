/*
  Yunagi 試作品の本体
  見た目と操作感を確かめるための見本。本番（Tauri＋CodeMirror 6）には流用しない。

  仕様の正本
  - 画面：docs/design/03-ui-spec.md
  - キー割当：docs/design/04-interaction-spec.md（2章、4.3）
  - 見た目と文言：docs/design/06-design-guide.md
*/
(function () {
  'use strict';

  const TODAY = window.YUNAGI_TODAY;
  const MSG = window.YUNAGI_MSG;
  const isMac = /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);
  const REVIEW_AT = 3; // 設定「操作」の持ち越し見直しの回数（既定3）
  const WARN_AT = REVIEW_AT; // 06 6章：持ち越しが見直しの回数以上なら --warn（見直しの回数と連動）
  const WORK_START = 9 * 60; // 凪メーターの始業（設定の勤務時間。既定 9:00）
  const WORK_END = 19 * 60;  // 終業（既定 19:00）
  const $ = (id) => document.getElementById(id);
  const CE = (() => {
    const d = document.createElement('div');
    d.contentEditable = 'plaintext-only';
    return d.contentEditable === 'plaintext-only' ? 'plaintext-only' : 'true';
  })();

  function h(tag, attrs, ...kids) {
    const el = document.createElement(tag);
    if (attrs) {
      for (const [k, v] of Object.entries(attrs)) {
        if (v == null || v === false) continue;
        if (k === 'class') el.className = v;
        else if (k.startsWith('on')) el.addEventListener(k.slice(2), v);
        else el.setAttribute(k, v === true ? '' : v);
      }
    }
    for (const c of kids.flat(Infinity)) if (c != null && c !== false) el.append(c.nodeType ? c : String(c));
    return el;
  }
  function stop(e) { e.preventDefault(); e.stopPropagation(); }

  /* 持ち越し回数の印：線で描いた ↻（どの書体でも同じ形に見えるように） */
  const rcTpl = (() => {
    const t = document.createElement('template');
    t.innerHTML = '<svg class="rc" viewBox="0 0 16 16" aria-hidden="true" focusable="false"><path d="M11.8 5.3A5 5 0 1 1 7.1 3.6"/><path d="M5.2 1.9 8.1 3.4 5.9 5.8"/></svg>';
    return t.content.firstChild;
  })();
  const carryIcon = () => rcTpl.cloneNode(true);
  const carryMark = (n) => [carryIcon(), h('span', { 'aria-hidden': 'true' }, String(n))];

  /* ---------- 日付 ---------- */

  const WD = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
  const parseD = (s) => { const [y, m, d] = s.split('-').map(Number); return new Date(Date.UTC(y, m - 1, d)); };
  const fmtD = (dt) => dt.toISOString().slice(0, 10);
  const addDays = (s, n) => { const d = parseD(s); d.setUTCDate(d.getUTCDate() + n); return fmtD(d); };
  const wdOf = (s) => WD[parseD(s).getUTCDay()];
  const shortDate = (s) => s.slice(5).replace('-', '.');
  const dateLabel = (s) => `${shortDate(s)} ${wdOf(s)}`;
  const fullDate = (s) => `${s.replace(/-/g, '.')} ${wdOf(s)}`;
  const diffDays = (a, b) => Math.round((parseD(a) - parseD(b)) / 86400000);

  /* ---------- 状態 ---------- */

  const S = {
    date: TODAY,
    mode: 'board',
    days: JSON.parse(JSON.stringify(window.YUNAGI_SAMPLE)),
    set: { layout: 'A', theme: 'system', lang: 'ja', fs: 15, noteMono: false, vim: false },
    ui: { overlay: null, pop: null, failMode: false, saveFail: false, saving: false, holdSaving: false, update: false, narrow: false, width: innerWidth, side: 'open', sideNarrowOpen: false, sideBeforeNote: null, clock: null, archFrom: 'board' },
    focus: {}, noteCaret: {}, lastSel: {},
    vim: 'normal', vimPend: null,
    editSession: null, memoEdit: null,
    carry: null, pal: null, palReturn: null, popReturn: null, ctx: null,
    arch: { sel: null },
  };
  for (const d of Object.values(S.days)) normDay(d);
  function normDay(d) {
    d.todos = d.todos || []; d.blockers = d.blockers || [];
    d.reflection = d.reflection || ['']; d.note = d.note || ['']; d.sends = d.sends || [];
    if (d.carryDone == null) d.carryDone = true;
    d.hist = { u: [], r: [] };
    return d;
  }
  const day = (date = S.date) => S.days[date] || (S.days[date] = normDay({}));
  const hasContent = (d) => d && (d.todos.length || d.blockers.length || d.note.some((t) => t.trim()) || d.reflection.some((t) => t.trim()));
  const M = () => MSG[S.set.lang];
  let seq = 0;
  const nid = (p) => `${p}${Date.now().toString(36)}${(++seq).toString(36)}`;
  const newItem = (kind, text, extra = {}) => ({ id: nid(kind === 'todo' ? 't' : 'b'), kind, text, status: 'open', carry: 0, origin: 'human', ...extra });
  const findItem = (id, date = S.date) => day(date).todos.find((i) => i.id === id) || day(date).blockers.find((i) => i.id === id);
  const listOf = (kind, date = S.date) => (kind === 'todo' ? day(date).todos : day(date).blockers);
  const isClosed = (it) => it.status === 'done' || it.status === 'resolved';

  /* ---------- キー（04 操作仕様書 2章） ---------- */

  const KEYS = {
    toggleMode: ['J'], prevDay: ['['], nextDay: [']'], today: ['T'], palette: ['K'], settings: [','],
    refTemplate: ['R', 1], sendTodo: ['T', 1], sendBlocker: ['B', 1], timeHeading: ['H', 1], exportReq: ['A', 1],
    sidebar: ['\\'], archive: ['L', 1],
  };
  function keyLabel(id) {
    const [k, sh] = KEYS[id];
    if (!isMac && S.set.vim && (id === 'prevDay' || id === 'nextDay')) return `Alt+${k}`; // 04 4.1
    return isMac ? `${sh ? '⇧' : ''}⌘${k}` : `Ctrl+${sh ? 'Shift+' : ''}${k}`;
  }
  const SC = [
    ['toggleMode', 'KeyJ'], ['today', 'KeyT'], ['palette', 'KeyK'], ['settings', 'Comma'],
    ['region1', 'Digit1'], ['region2', 'Digit2'], ['region3', 'Digit3'],
    ['refTemplate', 'KeyR', 1], ['sendTodo', 'KeyT', 1], ['sendBlocker', 'KeyB', 1], ['timeHeading', 'KeyH', 1], ['exportReq', 'KeyA', 1],
    ['archive', 'KeyL', 1],
  ];
  /* Mod（⌘ / Ctrl）が本来の割当。Alt は試作品だけの代わりのキー（ブラウザが先に奪うキーのため。README 参照） */
  function shortcutOf(e) {
    const mod = isMac ? e.metaKey : e.ctrlKey;
    const other = isMac ? e.ctrlKey : e.metaKey;
    const viaMod = mod && !e.altKey && !other;
    const viaAlt = e.altKey && !e.ctrlKey && !e.metaKey;
    if (!viaMod && !viaAlt) return null;
    let br = null;
    if (e.key === '[' || e.key === ']') br = e.key;
    else if (viaAlt && (e.code === 'BracketLeft' || e.code === 'BracketRight')) br = e.code === 'BracketLeft' ? '[' : ']';
    if (br && !e.shiftKey) {
      if (viaMod && S.set.vim && !isMac) return br === '[' ? 'vimEsc' : 'none'; // 04 4.1：Ctrl+[ は Vim の Esc
      return br === '[' ? 'prevDay' : 'nextDay';
    }
    if (!e.shiftKey && (e.key === '\\' || e.code === 'Backslash' || e.code === 'IntlYen' || e.code === 'IntlRo')) return 'sidebar';
    for (const [id, code, sh] of SC) if (e.code === code && !!sh === e.shiftKey) return id;
    return null;
  }
  function undoKind(e) {
    const mod = isMac ? e.metaKey : e.ctrlKey;
    if (!mod || e.altKey) return null;
    if (e.code === 'KeyZ') return e.shiftKey ? 'redo' : 'undo';
    if (!isMac && e.code === 'KeyY' && !e.shiftKey) return 'redo';
    return null;
  }

  /* ---------- 行エディタ（振り返り・ノート） ---------- */

  const refEd = new window.LineEditor($('refEd'), {
    onChange: (lines) => { day().reflection = lines; markDirty(); },
  });
  const noteEd = new window.LineEditor($('noteEd'), {
    onChange: (lines) => { day().note = lines; markDirty(); if (S.mode === 'note') renderOutline(); },
    markFor: (text) => { const s = day().sends.find((x) => x.text === text); return s ? s.kind : null; },
  });

  /* ---------- カーソル補助 ---------- */

  function textNodes(el) {
    const w = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
    const out = [];
    let t;
    while ((t = w.nextNode())) out.push(t);
    return out;
  }
  function setCaretIn(el, off) {
    const ts = textNodes(el);
    const r = document.createRange();
    let rem = Math.max(0, off);
    let done = false;
    for (let i = 0; i < ts.length && !done; i++) {
      if (rem <= ts[i].length) { r.setStart(ts[i], rem); done = true; } else rem -= ts[i].length;
    }
    if (!done) { if (ts.length) r.setStart(ts[ts.length - 1], ts[ts.length - 1].length); else r.setStart(el, 0); }
    r.collapse(true);
    const s = getSelection();
    s.removeAllRanges();
    s.addRange(r);
  }
  function caretOffset(el) {
    const s = getSelection();
    if (!s.rangeCount) return 0;
    const r = s.getRangeAt(0);
    if (!el.contains(r.startContainer)) return el.textContent.length;
    const pre = document.createRange();
    pre.selectNodeContents(el);
    pre.setEnd(r.startContainer, r.startOffset);
    return pre.toString().length;
  }
  function caretRect() {
    const s = getSelection();
    if (!s.rangeCount) return null;
    const r = s.getRangeAt(0).cloneRange();
    r.collapse(true);
    const rs = r.getClientRects();
    return rs.length ? rs[0] : null;
  }
  function onFirstLine(el) {
    const ls = el.getClientRects();
    if (ls.length <= 1) return true;
    const c = caretRect();
    return !c || c.top < ls[0].bottom - 1;
  }
  function onLastLine(el) {
    const ls = el.getClientRects();
    if (ls.length <= 1) return true;
    const c = caretRect();
    return !c || c.bottom > ls[ls.length - 1].top + 1;
  }

  /* ---------- 表示：設定・ヘッダー ---------- */

  const effLayout = () => (S.ui.width < 900 ? 'C' : S.set.layout); // 03 3章：幅 900px 未満は C
  function applySettings() {
    const r = document.documentElement;
    if (S.set.theme === 'system') r.removeAttribute('data-theme');
    else r.dataset.theme = S.set.theme;
    r.lang = S.set.lang;
    r.style.setProperty('--fs-setting', S.set.fs);
    $('app').classList.toggle('note-mono', S.set.noteMono);
    $('app').classList.toggle('narrow-sim', S.ui.narrow);
    applyLayout();
  }
  // サイドバー：幅 900px 未満では、明示的に開いたときだけ表示する
  const sideOpen = () => S.ui.side === 'open' && (S.ui.width >= 900 || S.ui.sideNarrowOpen);
  function applyLayout() {
    const l = effLayout();
    if ($('app').dataset.layout !== l) $('app').dataset.layout = l;
    const sd = sideOpen() ? 'open' : 'closed';
    if ($('app').dataset.side !== sd) $('app').dataset.side = sd;
    renderProtoStatus();
  }
  function toggleSidebar() {
    if (S.ui.width < 900) { S.ui.sideNarrowOpen = !sideOpen(); S.ui.side = 'open'; }
    else S.ui.side = sideOpen() ? 'closed' : 'open';
    applyLayout();
    renderHeader();
  }
  function renderStatic() {
    const L = M();
    $('hTodo').textContent = L.todo;
    $('hBlkTitle').textContent = L.blockers;
    $('hRef').textContent = L.reflection;
    $('segBoard').textContent = L.seg.board;
    $('segNote').textContent = L.seg.note;
    $('archTitle').textContent = L.nav.archive;
    $('searchLabel').textContent = L.searchLabel;
    $('searchKey').textContent = keyLabel('palette');
    $('prevDay').setAttribute('aria-label', L.withKey(L.cmdPrev, keyLabel('prevDay')));
    $('nextDay').setAttribute('aria-label', L.withKey(L.cmdNext, keyLabel('nextDay')));
    $('board').setAttribute('aria-label', L.boardMode);
    $('note').setAttribute('aria-label', L.noteMode);
    $('archive').setAttribute('aria-label', L.nav.archive);
    $('noteEd').setAttribute('aria-label', L.noteMode);
    $('refEd').setAttribute('aria-label', L.reflection);
  }
  const countUnseen = () => Object.values(S.days).reduce((n, d) => n + d.todos.concat(d.blockers).filter((i) => i.unseen).length, 0);

  /* その日の数字（凪メーター・最近の日・ふりかえりで共通） */
  function dayStats(date) {
    const d = S.days[date];
    if (!d) return null;
    const todos = d.todos.filter((i) => i.status !== 'dropped');
    const done = todos.filter((i) => i.status === 'done').length;
    const openT = todos.length - done;
    const openB = d.blockers.filter((i) => i.status !== 'resolved').length;
    const total = todos.length + d.blockers.length;
    const rest = openT + openB;
    return {
      n: todos.length, done, openT, openB, total, rest, nb: d.blockers.length,
      carried: todos.filter((i) => i.carry > 0).length, resolved: d.blockers.length - openB,
      ratio: total ? rest / total : 0, doneRatio: total ? (total - rest) / total : 0, any: !!hasContent(d),
    };
  }

  const SVGNS = 'http://www.w3.org/2000/svg';
  function svgEl(tag, attrs) {
    const el = document.createElementNS(SVGNS, tag);
    for (const [k, v] of Object.entries(attrs || {})) el.setAttribute(k, v);
    return el;
  }
  /* 波の線：幅 w、基準の高さ y0、振幅 amp、波の数 k、位相 ph */
  function wavePath(w, y0, amp, k, ph) {
    const pts = [];
    for (let i = 0; i <= 100; i++) {
      pts.push((i ? 'L' : 'M') + ((w * i) / 100).toFixed(1) + ' ' + (y0 + amp * Math.sin((i / 100) * Math.PI * 2 * k + ph)).toFixed(2));
    }
    return pts.join(' ');
  }
  function miniWave(ratio, calm) {
    const s = svgEl('svg', { viewBox: '0 0 40 12', preserveAspectRatio: 'none', 'aria-hidden': 'true' });
    if (calm) s.setAttribute('class', 'calm');
    s.append(svgEl('path', { d: wavePath(40, 6, calm ? 0 : 0.3 + 4 * ratio, 2, 0.4), 'vector-effect': 'non-scaling-stroke' }));
    return s;
  }
  /* 凪メーターの時刻（分）。今日はいまの時刻（試作品パネルで固定できる）、過去の日は終業、未来の日は始業 */
  function meterMinutes(date) {
    if (date < TODAY) return WORK_END;
    if (date > TODAY) return WORK_START;
    if (S.ui.clock != null) return S.ui.clock;
    const n = new Date();
    return n.getHours() * 60 + n.getMinutes();
  }
  const hhmm = (m) => `${Math.floor(m / 60)}:${String(m % 60).padStart(2, '0')}`;
  const sunT = (date) => Math.max(0, Math.min(1, (meterMinutes(date) - WORK_START) / (WORK_END - WORK_START)));

  /* 上部のバー */
  function renderHeader() {
    const L = M();
    const board = S.mode === 'board', note = S.mode === 'note', arch = S.mode === 'archive';
    $('hDate').replaceChildren(shortDate(S.date), h('span', { class: 'wd' }, wdOf(S.date)));
    $('dateNav').hidden = arch;
    $('modeSeg').hidden = arch;
    $('archTitle').hidden = !arch;
    $('segBoard').setAttribute('aria-pressed', String(board));
    $('segNote').setAttribute('aria-pressed', String(note));
    const tp = $('hToday');
    const isToday = S.date === TODAY;
    tp.className = 'today-pill' + (isToday ? ' is-today' : ' go');
    tp.replaceChildren(L.todayPill, ...(isToday ? [] : [h('kbd', null, keyLabel('today'))]));
    tp.setAttribute('aria-label', isToday ? L.todayPill : L.withKey(L.backToday, keyLabel('today')));
    tp.disabled = isToday;
    $('hUpdate').hidden = !(S.ui.update && !sideOpen()); // サイドバーを閉じているときは上部のバーに出す
    $('hUpdate').textContent = L.update;
    const un = countUnseen();
    const ag = $('hAgent');
    ag.hidden = un === 0;
    $('hAgentN').textContent = String(un);
    ag.setAttribute('aria-label', L.tipAgent(un));
    ag.title = L.tipAgent(un);
    const st = dayStats(S.date) || {};
    $('mTodo').textContent = st.openT ? L.leftN(st.openT) : '';
    $('mBlk').textContent = st.openB ? L.openN(st.openB) : '';
    renderStatus();
    renderSide();
    renderMeter();
    renderHints();
  }
  function renderStatus() {
    const L = M();
    const dot = $('hSaving');
    dot.hidden = !(S.ui.saving || S.ui.holdSaving);
    dot.title = L.saving;
    dot.setAttribute('aria-label', L.saving);
    const f = $('hSaveFail');
    f.hidden = !S.ui.saveFail;
    f.textContent = L.saveFail;
    const rows = [h('div', null, h('span', { class: 'dot' }), L.connected)];
    rows.push(S.ui.saveFail
      ? h('div', { class: 'warn' }, h('span', { class: 'dot warn' }), L.saveFail)
      : h('div', null, h('span', { class: 'dot hollow' }), L.savedLocal));
    if (S.ui.update) {
      const b = h('button', { type: 'button', class: 'nav-btn', style: 'padding:0;color:var(--accent-strong);font-size:inherit' }, L.update);
      b.addEventListener('click', () => togglePop('update'));
      rows.push(h('div', null, b));
    }
    $('sideStatus').replaceChildren(...rows);
  }

  /* サイドバー */
  const NAV_ICONS = {
    today: 'M2 11h12M5.2 11a2.8 2.8 0 0 1 5.6 0M8 4.2v1.4M3.9 6.1l1 1M12.1 6.1l-1 1',
    archive: 'M3 4.5h10v9H3zM3 7.5h10M5.8 3v3M10.2 3v3',
    inbox: 'M2.5 9.5 4.5 4h7l2 5.5V13h-11zM2.5 9.5H6l1 1.5h2l1-1.5h3.5',
    agents: 'M8 2.5 13.5 8 8 13.5 2.5 8z',
    settings: 'M8 5.5a2.5 2.5 0 1 1 0 5 2.5 2.5 0 0 1 0-5zM8 1.8v2M8 12.2v2M1.8 8h2M12.2 8h2M3.6 3.6 5 5M11 11l1.4 1.4M3.6 12.4 5 11M11 5l1.4-1.4',
  };
  function navIcon(k) {
    const s = svgEl('svg', { viewBox: '0 0 16 16', 'aria-hidden': 'true' });
    s.append(svgEl('path', { d: NAV_ICONS[k] }));
    return s;
  }
  function renderSide() {
    const L = M();
    const st = dayStats(TODAY) || {};
    const un = countUnseen();
    const items = [
      { k: 'today', label: L.nav.today, meta: st.rest ? L.leftN(st.rest) : '', cur: S.mode !== 'archive', run: () => goDate(TODAY) },
      { k: 'archive', label: L.nav.archive, meta: '', cur: S.mode === 'archive', run: () => { if (S.mode !== 'archive') openArchive(); } },
      { k: 'agents', label: L.nav.agents, meta: un ? String(un) : '', metaCls: 'agent', run: openHistory },
    ];
    $('nav').replaceChildren(...items.map((it) => {
      const b = h('button', { type: 'button', class: 'nav-btn', 'aria-current': it.cur ? 'page' : null, 'aria-disabled': it.disabled ? 'true' : null },
        navIcon(it.k), h('span', { class: 'nl' }, it.label), it.meta ? h('span', { class: 'nm' + (it.metaCls ? ' ' + it.metaCls : '') }, it.meta) : null);
      if (!it.disabled) b.addEventListener('click', it.run);
      return b;
    }));
    // 最近の日：今日と、項目やノートのある直近の日
    const dates = Object.keys(S.days).filter((d) => d <= TODAY && (d === TODAY || hasContent(S.days[d]))).sort().reverse().slice(0, 7);
    if (S.mode !== 'archive' && !dates.includes(S.date)) { dates.push(S.date); dates.sort().reverse(); }
    $('recent').replaceChildren(...dates.map((d) => {
      const s = dayStats(d) || { ratio: 0, rest: 0, total: 0, n: 0, done: 0 };
      const calm = s.total > 0 && s.rest === 0;
      const cur = S.mode === 'archive' ? d === S.arch.sel : d === S.date;
      const b = h('button', { type: 'button', class: 'recent-btn', 'aria-current': cur ? 'date' : null },
        miniWave(s.ratio, calm),
        h('span', { class: 'rl' }, dateLabel(d), d === TODAY ? h('span', { class: 'rt' }, L.todayPill) : null),
        h('span', { class: 'rc' }, s.n ? `${s.done}/${s.n}` : ''));
      b.addEventListener('click', () => {
        if (S.mode === 'archive') { S.arch.sel = d; renderArchive(); renderSide(); } else goDate(d);
      });
      return b;
    }));
    $('recentH').textContent = L.recent;
    $('navSettings').replaceChildren(navIcon('settings'), h('span', { class: 'nl' }, L.nav.settings), h('kbd', null, keyLabel('settings')));
  }

  /* 凪メーター：太陽 = 時刻、波 = 残りの仕事、光の道 = 終えた仕事 */
  function renderMeter() {
    const el = $('meter');
    if (S.mode === 'archive') { el.hidden = true; return; }
    el.hidden = false;
    const L = M();
    const full = S.mode === 'board';
    el.classList.toggle('compact', !full);
    const st = dayStats(S.date) || { rest: 0, total: 0, ratio: 0, doneRatio: 0, openT: 0, openB: 0, done: 0, n: 0, carried: 0 };
    const u = S.set.fs / 15;
    const H = (full ? 112 : 58) * u;
    const hz = (full ? 72 : 34) * u;
    const min = meterMinutes(S.date);
    const t = sunT(S.date);
    const size = (full ? 22 : 14) * u;
    const elev = t >= 1 ? -size / 2 : Math.sin(Math.PI * Math.max(0.06, t)) * (full ? 30 : 12) * u;
    const sunTop = hz - size - elev;
    const calm = st.total > 0 && st.rest === 0;
    const amp = calm || !st.total ? 0 : (0.6 + 5 * st.ratio) * (full ? 1 : 0.55);
    const waveH = H - hz - 2;
    const nPath = st.total && st.doneRatio > 0 ? Math.round(2 + 6 * st.doneRatio) : 0;
    const xPct = 4 + t * 92;
    const main = $('main');
    main.style.setProperty('--sun-t', String(xPct / 100));
    main.style.setProperty('--glow-y', `${main.querySelector('.top').offsetHeight + hz - 14 * u}px`);
    const waves = svgEl('svg', { class: 'waves', viewBox: `0 0 1000 ${waveH.toFixed(1)}`, preserveAspectRatio: 'none', 'aria-hidden': 'true' });
    waves.style.top = `${hz + 2}px`;
    waves.style.height = `${waveH}px`;
    [[0.22, 1, 9, 0, 0.85], [0.52, 0.7, 7, 1.3, 0.5], [0.8, 0.45, 11, 2.1, 0.28]].forEach(([y, a, k, ph, op]) => {
      waves.append(svgEl('path', { d: wavePath(1000, waveH * y, amp * a, k, ph), 'stroke-width': '1.2', opacity: String(op) }));
    });
    const kids = [
      h('div', { class: 'sea', style: `top:${hz}px` }),
      h('div', { class: 'sun-clip', style: `height:${hz}px` },
        h('div', { class: 'sun', style: `left:${xPct}%;top:${sunTop}px;width:${size}px;height:${size}px;margin-left:${-size / 2}px` })),
      h('div', { class: 'horizon', style: `top:${hz}px` }),
      waves,
    ];
    for (let i = 0; i < nPath; i++) {
      const w = Math.max(4, (full ? 44 : 26) - i * (full ? 8 : 6)) * u;
      kids.push(h('div', { class: 'path', style: `left:${xPct}%;top:${hz + (5 + i * (full ? 6 : 4)) * u}px;width:${w}px;margin-left:${-w / 2}px;opacity:${Math.max(0.12, 0.75 - i * 0.14)}` }));
    }
    const timeLabel = S.date < TODAY ? L.meter.dayEnd : (S.date > TODAY ? L.meter.notYet : hhmm(min));
    if (full) {
      // 残り（大きく）と、内訳・完了・持ち越しを1行に。凪のときは NAGI と完了数だけ
      const sub = calm
        ? [`${L.meter.done} `, h('b', null, `${st.done}/${st.n}`)]
        : [`${L.meter.todo} `, h('b', null, String(st.openT)), ` · ${L.meter.blockers} `, h('b', null, String(st.openB)),
          ` · ${L.meter.done} `, h('b', null, `${st.done}/${st.n}`),
          ...(st.carried ? [' · ', carryIcon(), ` ${L.meter.carried} `, h('b', null, String(st.carried))] : [])];
      kids.push(h('div', { class: 'm-info' },
        h('div', { class: 'm-time' }, timeLabel),
        h('div', { class: 'm-left' },
          calm ? h('span', { class: 'm-num calm' }, L.meter.nagi) : h('span', { class: 'm-num' }, String(st.rest)),
          calm ? null : h('span', { class: 'm-unit' }, L.meter.left),
          st.total ? h('span', { class: 'm-sub' }, sub) : null)));
    } else {
      kids.push(h('div', { class: 'm-info' }, h('span', { class: 'm-time' }, timeLabel),
        calm ? h('b', null, L.meter.nagi) : h('span', null, h('b', null, String(st.rest)), ` ${L.meter.left}`)));
    }
    el.replaceChildren(...kids);
    el.setAttribute('aria-label', L.meterAria(st.rest, st.openT, st.openB, st.done, st.n, st.carried));
  }

  /* 画面下のキーの案内 */
  function renderHints() {
    const L = M();
    const K = L.hint;
    const both = (a, b) => `${keyLabel(a)} ${keyLabel(b)}`;
    const list = S.mode === 'archive'
      ? [['← →', K.day], [both('prevDay', 'nextDay'), K.week], ['↵', K.open], ['Esc', K.back]]
      : S.mode === 'note'
        ? [[keyLabel('sendTodo'), K.toTodo], [keyLabel('sendBlocker'), K.toBlocker], [keyLabel('timeHeading'), K.heading], [keyLabel('toggleMode'), K.board], [keyLabel('sidebar'), K.sidebar]]
        : [[keyLabel('palette'), K.search], [keyLabel('toggleMode'), K.note], [isMac ? '⌘↵' : 'Ctrl+Enter', K.done], [both('prevDay', 'nextDay'), K.day], [keyLabel('archive'), K.archive], [keyLabel('sidebar'), K.sidebar]];
    $('hints').replaceChildren(...list.map(([k, l]) => h('span', null, h('kbd', null, k), l)));
  }

  /* ノートの右側：見出しの一覧と、送った行の数 */
  function renderOutline() {
    const L = M();
    const lines = day().note;
    const caret = S.noteCaret[S.date];
    const heads = [];
    lines.forEach((t, i) => {
      const m = /^#{1,3} (?:(\d{1,2}:\d{2})\s*)?(.*)$/.exec(t);
      if (m) heads.push({ i, time: m[1] || '', title: m[2] });
    });
    let cur = -1;
    if (caret) heads.forEach((hd, k) => { if (hd.i <= caret.line) cur = k; });
    const sends = day().sends.filter((s) => lines.includes(s.text));
    const nT = sends.filter((s) => s.kind === 'todo').length;
    const nB = sends.length - nT;
    $('outline').replaceChildren(
      h('div', { class: 'label' }, L.outline),
      ...heads.map((hd, k) => {
        const b = h('button', { type: 'button', class: k === cur ? 'cur' : null }, hd.time ? h('span', { class: 'ot' }, hd.time) : null, h('span', null, hd.title));
        b.addEventListener('click', () => noteEd.focus({ line: hd.i, offset: lines[hd.i].length }));
        return b;
      }),
      h('div', { class: 'label', style: 'margin-top:26px' }, L.sentLabel),
      h('div', { class: 'sent' }, `${L.hint.toTodo} `, h('b', null, String(nT)), h('br'), `${L.hint.toBlocker} `, h('b', null, String(nB))));
    const lnEls = noteEd.el.childNodes;
    heads.forEach((hd, k) => { const d = lnEls[hd.i]; if (d && d.classList) d.classList.toggle('cur', k === cur); });
  }

  /* ---------- ふりかえり（Archive）：過去の日を見返す ---------- */

  const weekStart = (d) => addDays(d, -((parseD(d).getUTCDay() + 6) % 7)); // 月曜
  function reflPick(date, idx) { // 振り返りの idx 番目の見出しの、最初の1行
    const d = S.days[date];
    if (!d) return '';
    let sec = -1;
    for (const t of d.reflection) {
      if (/^#{1,3} /.test(t)) { sec++; continue; }
      const v = t.replace(/^\s*[-*]\s*/, '').trim();
      if (sec === idx && v) return v;
    }
    return '';
  }
  function sceneEl(date) { // 1日の終わりの海（ふりかえりのカード）
    const st = dayStats(date) || { total: 0, ratio: 0, rest: 0, doneRatio: 0 };
    const fut = date > TODAY;
    const t = sunT(date);
    const calm = st.total > 0 && st.rest === 0;
    const amp = calm || !st.total ? 0 : 0.4 + 5 * st.ratio;
    const x = 6 + t * 88;
    const U = S.set.fs / 15;
    const hz = 42 * U;
    const kids = [];
    if (!fut && st.total) {
      const top = t >= 1 ? hz - 6 : hz - 12 - Math.sin(Math.PI * Math.max(0.06, t)) * 20 * U;
      kids.push(h('div', { class: 'sun-clip' }, h('div', { class: 'sun', style: `left:${x}%;top:${top}px` })));
    }
    kids.push(h('div', { class: 'horizon' }));
    const s = svgEl('svg', { viewBox: '0 0 200 24', preserveAspectRatio: 'none', 'aria-hidden': 'true' });
    s.append(svgEl('path', { d: wavePath(200, 7, amp, 4, 0.3), 'stroke-width': '1.1', opacity: '0.8' }),
      svgEl('path', { d: wavePath(200, 16, amp * 0.6, 5, 1.6), 'stroke-width': '1', opacity: '0.4' }));
    kids.push(s);
    const n = !fut && st.total && st.doneRatio > 0 ? Math.round(1 + 4 * st.doneRatio) : 0;
    for (let i = 0; i < n; i++) {
      const w = Math.max(4, 26 - i * 5);
      kids.push(h('div', { class: 'path', style: `left:${x}%;top:${hz + 4 + i * 4}px;width:${w}px;margin-left:${-w / 2}px;opacity:${Math.max(0.15, 0.7 - i * 0.14)}` }));
    }
    return h('div', { class: 'scene', 'aria-hidden': 'true' }, kids);
  }
  function selectArch(d) { S.arch.sel = d; renderArchive(); renderSide(); }
  function renderArchive() {
    const L = M();
    const A = L.arch;
    if (!S.arch.sel) S.arch.sel = prevDayWithContent(TODAY) || TODAY;
    const sel = S.arch.sel;
    const days5 = [0, 1, 2, 3, 4].map((i) => addDays(weekStart(sel), i));
    const sums = days5.map(dayStats).filter(Boolean);
    const tot = (k) => sums.reduce((n, x) => n + (x[k] || 0), 0);
    const head = h('div', { class: 'ar-head' },
      h('span', { class: 'label' }, A.week),
      h('span', { class: 'ar-range' }, `${shortDate(days5[0])} — ${shortDate(days5[4])}`),
      h('span', { class: 'ar-sum' }, h('b', null, String(tot('done'))), ` ${A.done} · `, h('b', null, String(tot('carried'))), ` ${A.carried} · `, h('b', null, String(tot('resolved'))), ` ${A.resolved}`));
    const week = h('div', { class: 'ar-week', role: 'listbox', 'aria-label': A.week }, days5.map((d) => {
      const st = dayStats(d);
      const fut = d > TODAY;
      const b = h('button', { type: 'button', class: 'ar-card' + (fut ? ' future' : ''), role: 'option', 'aria-selected': String(d === sel) },
        sceneEl(d),
        h('div', { class: 'ar-cap' }, h('span', null, shortDate(d)), h('span', { class: 'wd' }, wdOf(d)), d === TODAY ? h('span', { class: 'tg' }, L.todayPill) : null,
          h('span', { class: 'ct' }, st && st.n ? `${st.done}/${st.n}` : '—')));
      if (!fut) {
        b.addEventListener('click', () => selectArch(d));
        b.addEventListener('dblclick', () => leaveArchive(d));
      }
      return b;
    }));
    const dates = Object.keys(S.days).filter((d) => d <= TODAY && hasContent(S.days[d])).sort().reverse().slice(0, 8);
    const list = h('section', { style: 'min-width:0' }, h('div', { class: 'label' }, A.daily), dates.map((d) => {
      const st = dayStats(d);
      const b = h('button', { type: 'button', class: 'ar-entry', 'aria-selected': String(d === sel) },
        h('div', { class: 'eh' }, miniWave(st.ratio, st.total > 0 && st.rest === 0), h('b', null, dateLabel(d)),
          d === TODAY ? h('span', { style: 'font-size:11px;color:var(--accent-strong)' }, L.todayPill) : null,
          h('span', { class: 'ct' }, st.n ? `${st.done}/${st.n}` : '')),
        h('div', { class: 'el' }, h('span', { class: 'tag' }, A.good), reflPick(d, 0) || '—'),
        h('div', { class: 'el' }, h('span', { class: 'tag st' }, A.stuck), reflPick(d, 1) || '—'));
      b.addEventListener('click', () => selectArch(d));
      b.addEventListener('dblclick', () => leaveArchive(d));
      return b;
    }));
    const d = S.days[sel];
    const st = dayStats(sel);
    const open = h('button', { type: 'button', class: 'open' }, A.open, h('kbd', null, '↵'));
    open.addEventListener('click', () => leaveArchive(sel));
    const prev = h('section', { class: 'ar-prev' }, h('div', { class: 'ph' }, h('span', { class: 'pd' }, shortDate(sel), h('span', { class: 'wd' }, wdOf(sel))), open));
    if (!st || !st.any) prev.append(h('div', { class: 'ar-none' }, A.none));
    else {
      prev.append(h('div', { class: 'ps' }, A.stats(st.done, st.n, st.carried, st.resolved, st.nb)));
      const rcol = h('div', null, h('div', { class: 'label' }, L.reflection));
      for (const t of d.reflection) {
        if (/^#{1,3} /.test(t)) rcol.append(h('div', { class: 'ph2' }, t.replace(/^#{1,3} /, '')));
        else if (t.replace(/^\s*[-*]\s*/, '').trim()) rcol.append(h('div', { class: 'pl' }, t.replace(/^\s*[-*]\s*/, '')));
      }
      const ncol = h('div', null, h('div', { class: 'label' }, A.note));
      let cnt = 0;
      for (const t of d.note) {
        const m = /^#{1,3} (?:(\d{1,2}:\d{2})\s*)?(.*)$/.exec(t);
        if (m) { ncol.append(h('div', { class: 'nh' }, m[1] ? h('span', { class: 't' }, m[1]) : null, h('span', { class: 'n' }, m[2]))); cnt = 0; }
        else if (t.trim() && cnt < 2) { ncol.append(h('div', { class: 'pl mut' }, t.replace(/^\s*[-*]\s*/, ''))); cnt++; }
      }
      prev.append(h('div', { class: 'pcols' }, rcol, ncol));
    }
    $('archive').replaceChildren(head, week, h('div', { class: 'ar-cols' }, list, prev));
    if (S.mode === 'archive' && !$('archive').contains(document.activeElement) && !$('side').contains(document.activeElement)) $('archive').focus({ preventScroll: true });
  }
  function openArchive() {
    if (S.ui.overlay) return;
    saveFocusState();
    if (S.mode !== 'archive') S.ui.archFrom = S.mode;
    S.arch.sel = S.date !== TODAY ? S.date : (prevDayWithContent(TODAY) || TODAY);
    S.mode = 'archive';
    applyMode();
    renderArchive();
    $('archive').focus({ preventScroll: true });
  }
  function leaveArchive(date) {
    const back = S.ui.archFrom === 'note' ? 'note' : 'board';
    if (date !== S.date) {
      cleanupEmpty();
      S.date = date;
      S.editSession = null;
      S.memoEdit = null;
      loadDay();
    }
    S.mode = back;
    applyMode();
    if (back === 'note') noteEd.focus(S.noteCaret[S.date]);
    else restoreBoardFocus(S.focus[S.date]);
  }
  function archMove(n) {
    const d = addDays(S.arch.sel, n);
    selectArch(d > TODAY ? TODAY : d);
  }
  $('archive').addEventListener('keydown', (e) => {
    if (e.isComposing || e.defaultPrevented) return;
    const plain = !e.ctrlKey && !e.metaKey && !e.altKey;
    if (plain && (e.key === 'ArrowLeft' || e.key === 'ArrowRight')) { e.preventDefault(); archMove(e.key === 'ArrowLeft' ? -1 : 1); $('archive').focus({ preventScroll: true }); return; }
    if (plain && e.key === 'Enter') { e.preventDefault(); leaveArchive(S.arch.sel); return; }
    if (plain && e.key === 'Escape') { e.preventDefault(); leaveArchive(S.date); }
  });

  let flashT = 0;
  function flash(msg) {
    $('hFlash').textContent = msg;
    clearTimeout(flashT);
    flashT = setTimeout(() => { $('hFlash').textContent = ''; }, 2600);
  }

  /* ---------- 自動保存のまね（保存中の点・保存失敗） ---------- */

  let saveT = 0, retryT = 0;
  function markDirty() { clearTimeout(saveT); saveT = setTimeout(doSave, 400); }
  function doSave() {
    clearTimeout(retryT);
    S.ui.saving = true;
    renderStatus();
    setTimeout(() => {
      S.ui.saving = false;
      if (S.ui.failMode) { S.ui.saveFail = true; retryT = setTimeout(doSave, 5000); } // 自動で再試行
      else S.ui.saveFail = false;
      renderStatus();
      if (!S.ui.saveFail && S.ui.pop === 'save') closePop(true);
    }, 350);
  }

  /* ---------- S-01 整理モード：一覧 ---------- */

  const CHECK_D = 'M3.5 8.5 6.5 11.5 12.5 4.5';
  function itemEl(it) {
    const L = M();
    const closed = isClosed(it) || it.status === 'dropped';
    const li = h('li', { class: 'item' + (closed ? ' done' : '') + (it.status === 'dropped' ? ' dropped' : ''), 'data-id': it.id, tabindex: '-1' });
    if (it.unseen) li.append(h('span', { class: 'mk-agent lead', role: 'img', 'aria-label': L.agentUnseen, title: L.agentUnseen }));
    const cs = svgEl('svg', { viewBox: '0 0 16 16' });
    cs.append(svgEl('path', { d: CHECK_D }));
    li.append(h('span', { class: 'chk', 'aria-hidden': 'true' }, cs));
    const body = h('span', { class: 'body' });
    const txt = h('span', { class: 'txt', contenteditable: CE, spellcheck: 'false' });
    txt.textContent = it.text;
    body.append(txt);
    if (it.origin === 'agent') body.append(h('span', { class: 'agent-tag', title: L.agentItem }, h('span', { class: 'mk-agent' }), it.agentName || 'Agent'));
    if (it.kind === 'blocker' && it.status === 'resolved') {
      if (S.memoEdit === it.id) {
        const m = h('span', { class: 'memo-edit', contenteditable: CE, spellcheck: 'false', 'data-ph': L.memoPh, 'aria-label': L.memoPh });
        m.textContent = it.memo || '';
        body.append(m);
      } else if (it.memo) {
        body.append(h('span', { class: 'memo' }, `→ ${it.memo}`));
      }
    }
    li.append(body);
    if (it.status === 'dropped') li.append(h('span', { class: 'cstate' }, it.closeReason === 'split' ? L.splitDone : L.dropDone));
    if (it.carry > 0) li.append(h('span', { class: 'carry' + (it.carry >= WARN_AT ? ' warn' : ''), role: 'img', title: L.carryTimes(it.carry), 'aria-label': L.carryTimes(it.carry) }, carryMark(it.carry)));
    return li;
  }
  function renderLists() {
    const d = day();
    $('todoList').replaceChildren(...d.todos.map(itemEl));
    $('blkList').replaceChildren(...d.blockers.map(itemEl));
    $('todoEmpty').hidden = d.todos.length > 0;
    $('todoEmpty').textContent = S.date === TODAY ? M().emptyTodo : M().emptyOther; // 今日以外は「やることを書く」
    $('blkEmpty').hidden = d.blockers.length > 0;
  }
  const regionEl = (r) => document.querySelector(`#board .region[data-region="${r}"]`);
  const itemLi = (id) => document.querySelector(`#board .item[data-id="${CSS.escape(id)}"]`);

  function focusItem(id, offset = 'end', insert = false) {
    const li = itemLi(id);
    if (!li) return;
    if (S.set.vim && S.vim === 'normal' && !insert) {
      li.focus({ preventScroll: true });
    } else {
      const t = li.querySelector('.txt');
      t.focus({ preventScroll: true });
      setCaretIn(t, offset === 'end' || offset == null ? t.textContent.length : offset);
    }
    li.scrollIntoView({ block: 'nearest' });
  }
  function focusMemo(id) {
    const li = itemLi(id);
    const m = li && li.querySelector('.memo-edit');
    if (!m) return;
    m.focus();
    setCaretIn(m, m.textContent.length);
  }
  function focusRegion(r) {
    if (S.mode !== 'board') return;
    if (r === 'reflection') {
      const f = S.focus[S.date];
      refEd.focus(f && f.region === 'reflection' ? f.caret : null);
      return;
    }
    const items = listOf(r);
    if (!items.length) { regionEl(r).focus(); return; }
    const last = S.lastSel[S.date + r];
    const it = items.find((i) => i.id === last) || items.find((i) => i.status === 'open') || items[0];
    focusItem(it.id, 'end');
  }
  function currentFocus() {
    const a = document.activeElement;
    if (!a || a === document.body) return null;
    const li = a.closest && a.closest('#board .item');
    if (li) {
      const t = li.querySelector('.txt');
      return { region: li.closest('.region').dataset.region, id: li.dataset.id, offset: a === t ? caretOffset(t) : 'end', insert: a !== li, memo: a.classList.contains('memo-edit') };
    }
    if (refEd.el.contains(a)) return { region: 'reflection', caret: refEd.getCaret() };
    if (a.matches && a.matches('#board .region')) return { region: a.dataset.region };
    return null;
  }
  function restoreBoardFocus(f) {
    if (S.mode === 'note') { noteEd.focus(S.noteCaret[S.date]); return; }
    if (S.mode === 'archive') { $('archive').focus({ preventScroll: true }); return; }
    if (!f) { focusRegion('todo'); return; }
    if (f.region === 'reflection') { refEd.focus(f.caret); return; }
    if (f.id && findItem(f.id)) {
      if (f.memo && S.memoEdit === f.id) focusMemo(f.id);
      else focusItem(f.id, f.offset, f.insert);
      return;
    }
    focusRegion(f.region || 'todo');
  }
  function trackFocus() {
    const a = document.activeElement;
    if (!a || a === document.body) return;
    if (noteEd.el.contains(a)) {
      const c = noteEd.getCaret();
      if (c) { S.noteCaret[S.date] = c; if (S.mode === 'note') renderOutline(); }
      return;
    }
    const f = currentFocus();
    if (!f) return;
    S.focus[S.date] = f;
    if (f.id) S.lastSel[S.date + f.region] = f.id;
  }
  document.addEventListener('selectionchange', trackFocus);
  document.addEventListener('focusin', (e) => {
    trackFocus();
    if (S.set.vim) {
      const t = e.target;
      if (t.classList && (t.classList.contains('txt') || t.classList.contains('memo-edit'))) S.vim = 'insert';
      else if (t.classList && (t.classList.contains('item') || t.classList.contains('region'))) S.vim = 'normal';
    }
    renderVimInd();
  });

  /* 取り消し（整理モードのリスト：追加・削除・完了・並べ替え・文字の編集） */
  const snapshot = () => { const d = day(); return JSON.stringify({ todos: d.todos, blockers: d.blockers, f: currentFocus() || S.focus[S.date] }); };
  function pushHist() {
    const d = day();
    d.hist.u.push(snapshot());
    if (d.hist.u.length > 200) d.hist.u.shift();
    d.hist.r = [];
  }
  function applySnap(json) {
    const d = day();
    const s = JSON.parse(json);
    d.todos = s.todos;
    d.blockers = s.blockers;
    S.editSession = null;
    S.memoEdit = null;
    renderLists();
    renderHeader();
    markDirty();
    restoreBoardFocus(s.f);
  }
  function boardUndo() { const d = day(); if (!d.hist.u.length) return; d.hist.r.push(snapshot()); applySnap(d.hist.u.pop()); }
  function boardRedo() { const d = day(); if (!d.hist.r.length) return; d.hist.u.push(snapshot()); applySnap(d.hist.r.pop()); }

  function afterStructural() { S.editSession = null; renderLists(); renderHeader(); markDirty(); }
  function addAt(kind, index, text = '') {
    pushHist();
    const list = listOf(kind);
    const it = newItem(kind, text);
    list.splice(index == null ? list.length : index, 0, it);
    afterStructural();
    S.editSession = it.id;
    focusItem(it.id, 0, true);
  }
  function splitAt(it, off) {
    const list = listOf(it.kind);
    const i = list.indexOf(it);
    if (off === 0 && it.text.length > 0) { addAt(it.kind, i); return; } // 行頭の Enter は直上に追加
    pushHist();
    const rest = it.text.slice(off);
    it.text = it.text.slice(0, off);
    const n = newItem(it.kind, rest);
    list.splice(i + 1, 0, n);
    afterStructural();
    S.editSession = n.id;
    focusItem(n.id, 0, true);
  }
  function toggleDone(it) {
    if (it.status === 'dropped') return;
    pushHist();
    if (it.kind === 'todo') it.status = it.status === 'done' ? 'open' : 'done';
    else it.status = it.status === 'resolved' ? 'open' : 'resolved';
    const resolving = it.kind === 'blocker' && it.status === 'resolved';
    S.memoEdit = resolving ? it.id : null;
    afterStructural();
    if (resolving) focusMemo(it.id); // 解消時に一言メモ（F-03）
    else focusItem(it.id, 'end');
  }
  function deleteItem(it, dir) {
    pushHist();
    const list = listOf(it.kind);
    const i = list.indexOf(it);
    list.splice(i, 1);
    afterStructural();
    const j = Math.min(i, list.length) - (dir === 'prev' ? 1 : 0);
    const n = list[Math.max(0, j)] || list[list.length - 1];
    if (n) focusItem(n.id, 'end');
    else regionEl(it.kind).focus();
  }
  function moveItem(it, dir) {
    const list = listOf(it.kind);
    const i = list.indexOf(it);
    const j = i + dir;
    if (j < 0 || j >= list.length) return;
    const f = currentFocus();
    pushHist();
    list.splice(i, 1);
    list.splice(j, 0, it);
    afterStructural();
    focusItem(it.id, f ? f.offset : 'end', f ? f.insert : false);
  }
  function moveFocus(it, dir, off) {
    const list = listOf(it.kind);
    const n = list[list.indexOf(it) + dir];
    if (n) focusItem(n.id, Math.min(off, n.text.length));
  }
  function cleanupEmpty(exceptId) {
    const d = day();
    let changed = false;
    for (const k of ['todos', 'blockers']) {
      const n = d[k].length;
      d[k] = d[k].filter((i) => i.text.trim() !== '' || i.id === exceptId);
      if (d[k].length !== n) changed = true;
    }
    return changed;
  }

  /* ---------- 整理モードのイベント ---------- */

  const board = $('board');
  board.addEventListener('beforeinput', (e) => {
    const t = e.target;
    if (!t.classList || !(t.classList.contains('txt') || t.classList.contains('memo-edit'))) return;
    const li = t.closest('.item');
    const it = li && findItem(li.dataset.id);
    if (!it) return;
    const tp = e.inputType || '';
    if (tp === 'insertParagraph' || tp === 'insertLineBreak') { e.preventDefault(); return; }
    if (tp === 'historyUndo') { e.preventDefault(); boardUndo(); return; }
    if (tp === 'historyRedo') { e.preventDefault(); boardRedo(); return; }
    if (tp.startsWith('format')) { e.preventDefault(); return; }
    if (S.editSession !== it.id) { pushHist(); S.editSession = it.id; }
  });
  board.addEventListener('input', (e) => {
    const t = e.target;
    const li = t.closest && t.closest('.item');
    const it = li && findItem(li.dataset.id);
    if (!it) return;
    let v = t.textContent;
    if (/\n/.test(v)) { v = v.replace(/\n/g, ' '); t.textContent = v; setCaretIn(t, v.length); }
    if (t.classList.contains('txt')) it.text = v;
    else if (t.classList.contains('memo-edit')) it.memo = v;
    markDirty();
  });
  board.addEventListener('paste', (e) => {
    const t = e.target;
    if (!t.closest || !t.closest('.txt, .memo-edit')) return;
    e.preventDefault();
    const text = (e.clipboardData.getData('text/plain') || '').replace(/\s*\n\s*/g, ' ');
    document.execCommand('insertText', false, text);
  });
  board.addEventListener('focusout', (e) => {
    const li = e.target.closest && e.target.closest('.item');
    if (!li) return;
    const id = li.dataset.id;
    setTimeout(() => {
      if (li.isConnected && li.contains(document.activeElement)) return;
      const f = currentFocus();
      let changed = false;
      if (S.memoEdit === id && !(f && f.memo && f.id === id)) { S.memoEdit = null; changed = true; }
      if (cleanupEmpty(f && f.id)) changed = true;
      if (changed) {
        renderLists();
        renderHeader();
        if (f && f.id) restoreBoardFocus(f);
        else if (f && f.region && f.region !== 'reflection') regionEl(f.region).focus();
      }
    }, 0);
  });
  board.addEventListener('mousedown', (e) => {
    if (e.button !== 0) return;
    const li = e.target.closest('.item');
    if (li) {
      const id = li.dataset.id;
      if (e.target.closest('.chk')) { e.preventDefault(); const it = findItem(id); if (it) toggleDone(it); return; }
      if (e.target.closest('.memo') && !e.target.closest('.memo-edit')) {
        e.preventDefault();
        S.memoEdit = id;
        renderLists();
        focusMemo(id);
        return;
      }
      const onText = e.target.closest('.txt, .memo-edit');
      if (S.set.vim && S.vim === 'normal' && onText) { e.preventDefault(); li.focus(); return; } // 04 5章：クリックで選択
      if (!onText) { e.preventDefault(); startDrag(e, li); }
      return;
    }
    if (e.target.closest('#todoEmpty')) { e.preventDefault(); addAt('todo', null); return; }
    if (e.target.closest('#blkEmpty')) { e.preventDefault(); addAt('blocker', null); return; }
    const reg = e.target.closest('.region');
    if (!reg) return;
    if (reg.dataset.region === 'reflection') {
      if (!refEd.el.contains(e.target)) { e.preventDefault(); refEd.focus(); }
      return;
    }
    e.preventDefault();
    focusRegion(reg.dataset.region);
  });
  board.addEventListener('dblclick', (e) => {
    if (!S.set.vim) return;
    const li = e.target.closest('.item');
    if (li) focusItem(li.dataset.id, 'end', true); // 04 5章：ダブルクリックで編集
  });
  board.addEventListener('keydown', (e) => {
    if (e.isComposing || e.keyCode === 229 || e.defaultPrevented) return;
    const t = e.target;
    if (refEd.el.contains(t)) return;
    const mod = isMac ? e.metaKey : e.ctrlKey;
    const plain = !mod && !e.altKey && !(isMac && e.ctrlKey);
    if (S.set.vim && t.classList && (t.classList.contains('item') || t.classList.contains('region'))) { vimKey(e, t); return; }
    if (t.classList && t.classList.contains('region')) {
      if (e.key === 'Enter' && plain) { e.preventDefault(); addAt(t.dataset.region, null); }
      return;
    }
    const li = t.closest && t.closest('.item');
    const it = li && findItem(li.dataset.id);
    if (!it) return;
    if (t.classList.contains('memo-edit')) {
      if ((e.key === 'Enter' && !mod) || e.key === 'Escape') {
        e.preventDefault();
        S.memoEdit = null;
        renderLists();
        focusItem(it.id, 'end', !(S.set.vim && e.key === 'Escape'));
      }
      return;
    }
    if (!t.classList.contains('txt')) return;
    if (e.key === 'Enter' && mod && !e.altKey && !e.shiftKey) { e.preventDefault(); toggleDone(it); return; }
    if (e.key === 'Enter' && plain && !e.shiftKey) { e.preventDefault(); splitAt(it, caretOffset(t)); return; }
    if (e.key === 'Backspace' && mod && !e.altKey) { e.preventDefault(); deleteItem(it, 'next'); return; }
    if (e.altKey && !mod && !e.shiftKey && (e.key === 'ArrowUp' || e.key === 'ArrowDown')) { e.preventDefault(); moveItem(it, e.key === 'ArrowUp' ? -1 : 1); return; }
    if (plain && !e.shiftKey && e.key === 'ArrowUp' && onFirstLine(t)) { e.preventDefault(); moveFocus(it, -1, caretOffset(t)); return; }
    if (plain && !e.shiftKey && e.key === 'ArrowDown' && onLastLine(t)) { e.preventDefault(); moveFocus(it, 1, caretOffset(t)); return; }
    if (plain && e.key === 'Backspace' && t.textContent === '' && getSelection().isCollapsed) { e.preventDefault(); deleteItem(it, 'prev'); return; }
    if (e.key === 'Escape' && S.set.vim) { e.preventDefault(); S.vim = 'normal'; li.focus(); }
  });

  /* マウスでの並べ替え（04 5章） */
  function startDrag(e, li) {
    const id = li.dataset.id;
    const it = findItem(id);
    const ul = li.parentElement;
    const y0 = e.clientY;
    let dragging = false, target = null, ind = null;
    const els = () => [...ul.querySelectorAll(':scope > .item')];
    const onMove = (ev) => {
      if (!dragging && Math.abs(ev.clientY - y0) < 5) return;
      if (!dragging) { dragging = true; li.classList.add('dragging'); ind = h('div', { class: 'drop-ind' }); ul.append(ind); }
      const list = els();
      let pos = list.length;
      for (let k = 0; k < list.length; k++) {
        const r = list[k].getBoundingClientRect();
        if (ev.clientY < r.top + r.height / 2) { pos = k; break; }
      }
      target = pos;
      const ur = ul.getBoundingClientRect();
      const y = list[pos] ? list[pos].getBoundingClientRect().top : (list.length ? list[list.length - 1].getBoundingClientRect().bottom : ur.top);
      ind.style.top = `${y - ur.top - 1}px`;
    };
    const onUp = () => {
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
      if (!dragging) { focusItem(id, 'end'); return; }
      li.classList.remove('dragging');
      ind.remove();
      const list = listOf(it.kind);
      const shown = els().map((x) => x.dataset.id);
      const from = shown.indexOf(id);
      let to = target;
      if (to > from) to -= 1;
      if (to !== from) {
        pushHist();
        const refId = shown.filter((x) => x !== id)[to];
        list.splice(list.indexOf(it), 1);
        const at = refId ? list.findIndex((x) => x.id === refId) : list.length;
        list.splice(at, 0, it);
        markDirty();
      }
      renderLists();
      focusItem(id, 'end');
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
  }

  /* ---------- Vimモード（04 4.3 整理モードのリスト） ---------- */

  function renderVimInd() {
    const el = $('vimInd');
    const a = document.activeElement;
    const inList = a && a.closest && a.closest('#board .item, #board .region[data-region="todo"], #board .region[data-region="blocker"]');
    el.hidden = !(S.set.vim && S.mode === 'board' && !S.ui.overlay && inList);
    el.textContent = S.vim === 'insert' ? M().vimInsert : M().vimNormal;
  }
  function vimOpen(region, it, after) {
    const list = listOf(region);
    const i = it ? list.indexOf(it) + after : list.length;
    addAt(region, i);
  }
  function cycleRegion(r, dir) {
    const order = ['todo', 'blocker', 'reflection'];
    focusRegion(order[(order.indexOf(r) + dir + order.length) % order.length]);
  }
  function vimKey(e, t) {
    const mod = isMac ? e.metaKey : e.ctrlKey;
    const region = t.closest('.region').dataset.region;
    const li = t.classList.contains('item') ? t : null;
    const it = li && findItem(li.dataset.id);
    const list = listOf(region);
    const idx = it ? list.indexOf(it) : -1;
    const k = e.key;
    if (e.ctrlKey && !e.metaKey && !e.altKey && k === 'r') { stop(e); boardRedo(); return; }
    if (e.altKey && !mod && it && (k === 'ArrowUp' || k === 'ArrowDown')) { stop(e); moveItem(it, k === 'ArrowUp' ? -1 : 1); return; }
    if (mod && !e.altKey && it && k === 'Enter') { stop(e); toggleDone(it); return; }
    if (mod && !e.altKey && it && k === 'Backspace') { stop(e); deleteItem(it, 'next'); return; }
    if (mod || e.altKey || (isMac && e.ctrlKey)) return;
    const pend = S.vimPend;
    S.vimPend = null;
    const go = (j) => { const n = list[Math.max(0, Math.min(list.length - 1, j))]; if (n) focusItem(n.id); };
    switch (k) {
      case 'j': case 'ArrowDown': stop(e); if (it) go(idx + 1); return;
      case 'k': case 'ArrowUp': stop(e); if (it) go(idx - 1); return;
      case 'g': stop(e); if (pend === 'g') go(0); else S.vimPend = 'g'; return;
      case 'G': stop(e); go(list.length - 1); return;
      case 'o': stop(e); vimOpen(region, it, 1); return;
      case 'O': stop(e); vimOpen(region, it, 0); return;
      case 'i': stop(e); if (it) focusItem(it.id, 0, true); return;
      case 'a': stop(e); if (it) focusItem(it.id, 'end', true); return;
      case 'x': stop(e); if (it) toggleDone(it); return;
      case 'd': stop(e); if (pend === 'd' && it) deleteItem(it, 'next'); else S.vimPend = 'd'; return;
      case 'J': stop(e); if (it) moveItem(it, 1); return;
      case 'K': stop(e); if (it) moveItem(it, -1); return;
      case 'u': stop(e); boardUndo(); return;
      case 'Tab': stop(e); cycleRegion(region, e.shiftKey ? -1 : 1); return;
      case 'Enter': stop(e); if (!it) vimOpen(region, null, 1); else focusItem(it.id, 'end', true); return;
      case 'Escape': stop(e); return;
      default:
        if (k.length === 1) stop(e); // ノーマルモードでは文字を入力しない（04 4.4）
    }
  }
  function vimEscape() {
    const a = document.activeElement;
    const li = a && a.closest && a.closest('#board .item');
    if (li && a !== li) { S.memoEdit = null; S.vim = 'normal'; li.focus(); }
    return true;
  }

  /* ---------- モード・日付の移動 ---------- */

  function saveFocusState() {
    if (S.mode === 'board') { const f = currentFocus(); if (f) S.focus[S.date] = f; }
    else if (S.mode === 'note') { const c = noteEd.getCaret(); if (c) S.noteCaret[S.date] = c; }
  }
  function applyMode() {
    // ノートに入るとサイドバーを閉じて書くことに集中し、出ると元に戻す
    if (S.mode === 'note' && S.ui.sideBeforeNote == null) { S.ui.sideBeforeNote = S.ui.side; S.ui.side = 'closed'; }
    if (S.mode !== 'note' && S.ui.sideBeforeNote != null) { S.ui.side = S.ui.sideBeforeNote; S.ui.sideBeforeNote = null; }
    $('app').dataset.mode = S.mode;
    $('board').hidden = S.mode !== 'board';
    $('note').hidden = S.mode !== 'note';
    $('archive').hidden = S.mode !== 'archive';
    applyLayout();
    renderHeader();
    if (S.mode === 'note') renderOutline();
    renderVimInd();
  }
  function loadDay() {
    const d = day();
    renderLists();
    refEd.setText(d.reflection);
    noteEd.setText(d.note);
  }
  function switchTo(date, mode) {
    saveFocusState();
    if (date !== S.date) {
      cleanupEmpty();
      S.date = date;
      S.editSession = null;
      S.memoEdit = null;
      loadDay();
    }
    if (mode !== S.mode) { S.mode = mode; }
    applyMode();
  }
  function toggleMode() { // 03 4章：瞬時に切り替え、双方のカーソル位置を保持
    if (S.mode === 'archive') { leaveArchive(S.date); return; }
    switchTo(S.date, S.mode === 'board' ? 'note' : 'board');
    if (S.mode === 'note') noteEd.focus(S.noteCaret[S.date]);
    else restoreBoardFocus(S.focus[S.date]);
  }
  function goDate(d) {
    if (S.mode === 'archive') { leaveArchive(d); return; }
    switchTo(d, S.mode);
    if (S.mode === 'note') noteEd.focus(S.noteCaret[d]);
    else restoreBoardFocus(S.focus[d]);
  }
  function restoreAfterOverlay(f) {
    if (S.mode === 'archive') { $('archive').focus({ preventScroll: true }); return; }
    if (S.mode === 'note') noteEd.focus(S.noteCaret[S.date]);
    else restoreBoardFocus(f || S.focus[S.date]);
  }
  function setOverlayOpen(on) {
    $('app').classList.toggle('has-ov', on);
    $('page').inert = on;
    renderVimInd();
  }

  /* ---------- 振り返り・ノートの操作 ---------- */

  function insertTemplate() { // 04 2章 Mod+Shift+R（振り返り）
    if (!refEd.el.contains(document.activeElement)) return;
    const lines = refEd.texts();
    const s = refEd.getSel();
    const L0 = s ? s.end.line : lines.length - 1;
    const first = lines[L0].trim() === '' ? L0 : L0 + 1;
    refEd.insertLinesAtCaret(M().template);
    refEd.setCaret({ line: first + 1, offset: 2 });
  }
  function insertTimeHeading() { // 04 2章 Mod+Shift+H（ノート）
    if (!noteEd.el.contains(document.activeElement)) noteEd.focus(S.noteCaret[S.date]);
    const now = new Date();
    const head = `## ${String(now.getHours()).padStart(2, '0')}:${String(now.getMinutes()).padStart(2, '0')} `;
    const lines = noteEd.texts();
    const s = noteEd.getSel();
    const L0 = s ? s.end.line : lines.length - 1;
    noteEd.checkpoint('struct');
    if (lines[L0].trim() === '') noteEd.replace(L0, 0, L0, lines[L0].length, head);
    else noteEd.replace(L0, lines[L0].length, L0, lines[L0].length, `\n\n${head}`);
  }
  function sendRange(kind, range) { // F-06：各行を1件ずつ送る。原文は残し、送信済みの印を付ける
    if (!range) return;
    const d = day();
    const lines = noteEd.texts();
    for (let i = range[0]; i <= range[1]; i++) {
      const raw = lines[i];
      if (!raw || !raw.trim() || /^#{1,3} /.test(raw)) continue;
      if (d.sends.some((s) => s.text === raw)) continue;
      const text = raw.replace(/^\s*[-*]\s+/, '').trim();
      if (!text) continue;
      const it = newItem(kind, text, { origin: 'note' });
      listOf(kind).push(it);
      d.sends.push({ kind, text: raw, itemId: it.id });
    }
    noteEd.refreshMarks();
    noteEd.lastKind = null; // 送る操作を取り消しの区切りにする
    renderLists();
    renderHeader();
    markDirty();
  }
  function sendLines(kind) {
    if (!noteEd.el.contains(document.activeElement)) return;
    sendRange(kind, noteEd.selectedLineRange());
  }

  /* ノートの行の右クリックメニュー（04 5章） */
  noteEd.el.addEventListener('contextmenu', (e) => {
    const ln = e.target.closest('.ln');
    if (!ln) return;
    const idx = [...noteEd.el.childNodes].indexOf(ln);
    const text = ln.textContent;
    if (!text.trim() || /^#{1,3} /.test(text)) return;
    e.preventDefault();
    const r = noteEd.selectedLineRange();
    const inSel = r && idx >= r[0] && idx <= r[1] && !getSelection().isCollapsed;
    const range = inSel ? r : [idx, idx];
    const L = M();
    const m = $('ctx');
    const item = (label, key, kind) => h('button', { type: 'button', onclick: () => { closeCtx(true); sendRange(kind, range); } }, label, h('kbd', null, keyLabel(key)));
    m.replaceChildren(item(L.sendTodo, 'sendTodo', 'todo'), item(L.sendBlocker, 'sendBlocker', 'blocker'));
    m.style.left = `${Math.min(e.clientX, innerWidth - 200)}px`;
    m.style.top = `${Math.min(e.clientY, innerHeight - 90)}px`;
    m.hidden = false;
    m.querySelector('button').focus();
  });
  function closeCtx(restore) {
    $('ctx').hidden = true;
    if (restore) noteEd.focus(S.noteCaret[S.date]);
  }

  /* 依頼文の書き出し（04 2章 Mod+Shift+A、A-06） */
  async function copyText(t) {
    try { await navigator.clipboard.writeText(t); return true; } catch (_) { /* 次の方法へ */ }
    try {
      const ta = h('textarea', { style: 'position:fixed;left:-9999px;top:0' });
      ta.value = t;
      document.body.append(ta);
      ta.select();
      const ok = document.execCommand('copy');
      ta.remove();
      return ok;
    } catch (_) { return false; }
  }
  function exportRequest(f) {
    const L = M();
    f = f || currentFocus();
    const it = f && f.region === 'todo' && f.id && findItem(f.id);
    if (!it) { flash(L.noTodo); return; }
    let text = L.request(it.text);
    const d = day();
    const send = d.sends.find((s) => s.itemId === it.id);
    const i = send ? d.note.indexOf(send.text) : -1;
    if (i >= 0) {
      const isH = (t) => /^#{1,3} /.test(t);
      let a = i;
      while (a > 0 && !isH(d.note[a])) a--;
      const head = isH(d.note[a]) ? d.note[a].replace(/^#{1,3} /, '') : null;
      let b = i + 1;
      while (b < d.note.length && !isH(d.note[b])) b++;
      const sec = d.note.slice(head ? a + 1 : a, b).filter((t) => t.trim());
      text += `\n\n${L.requestNote(S.date, head)}\n${sec.join('\n')}`;
    }
    copyText(text).then((ok) => {
      flash(ok ? L.copied : L.copyFail);
      if (S.mode === 'board' && (!document.activeElement || document.activeElement === document.body)) restoreBoardFocus(f);
    });
  }
  function openHistory() { // S-06 は作らない。開いたことにして未確認の印を消す
    for (const d of Object.values(S.days)) for (const i of d.todos.concat(d.blockers)) i.unseen = false;
    renderLists();
    renderHeader();
    flash(M().protoNoHistory);
  }

  /* ---------- ヘッダーのポップオーバー（保存失敗の詳細、新しい版） ---------- */

  function openPop(kind) {
    const L = M();
    const p = $('pop');
    S.ui.pop = kind;
    if (kind === 'save') {
      p.replaceChildren(
        h('p', null, L.saveCause),
        h('p', { class: 'path' }, '~/Documents/Yunagi'),
        h('p', { class: 'sub' }, L.saveKeep),
        h('div', { class: 'acts' },
          h('button', { type: 'button', class: 'btn primary', onclick: () => { S.ui.failMode = false; renderProto(); doSave(); } }, L.retry),
          h('button', { type: 'button', class: 'btn', onclick: () => flash(L.protoNoDir) }, L.changeDir)));
      p.setAttribute('aria-label', L.saveFail);
    } else {
      p.replaceChildren(
        h('p', null, L.updateTitle('0.5.0')),
        h('ul', null, L.updateNotes.map((n) => h('li', null, n))),
        h('div', { class: 'acts' },
          h('button', { type: 'button', class: 'btn primary', onclick: () => { closePop(true); flash(L.protoNoUpdate); } }, L.updateRestart),
          h('button', { type: 'button', class: 'btn', onclick: () => closePop(true) }, L.later)));
      p.setAttribute('aria-label', L.update);
    }
    p.setAttribute('role', 'dialog');
    p.hidden = false;
    p.querySelector('button').focus();
  }
  function closePop(restore) {
    if (!S.ui.pop) return;
    $('pop').hidden = true;
    S.ui.pop = null;
    if (restore) restoreAfterOverlay(S.popReturn);
  }
  function togglePop(kind) {
    if (S.ui.pop === kind) { closePop(true); return; }
    S.popReturn = S.focus[S.date];
    openPop(kind);
  }
  $('hSaveFail').addEventListener('click', () => togglePop('save'));
  $('prevDay').addEventListener('click', () => goDate(addDays(S.date, -1)));
  $('nextDay').addEventListener('click', () => goDate(addDays(S.date, 1)));
  $('segBoard').addEventListener('click', () => { if (S.mode !== 'board') toggleMode(); });
  $('segNote').addEventListener('click', () => { if (S.mode !== 'note') toggleMode(); });
  $('searchBtn').addEventListener('click', () => openPalette());
  $('hAgent').addEventListener('click', () => openHistory());
  $('hUpdate').addEventListener('click', () => togglePop('update'));
  $('navSettings').addEventListener('click', () => flash(M().protoNoSettings));
  $('hToday').addEventListener('click', () => goDate(TODAY));
  document.addEventListener('mousedown', (e) => {
    if (!$('ctx').hidden && !$('ctx').contains(e.target)) closeCtx(false);
    if (S.ui.pop && !$('pop').contains(e.target) && !e.target.closest('#hSaveFail, #hUpdate, #sideStatus')) closePop(false);
  }, true);

  /* ---------- S-03 持ち越し ---------- */

  function prevDayWithContent(date) { // 前回の日：今日より前で、項目またはノートが存在する直近の日
    const ds = Object.keys(S.days).filter((d) => d < date && hasContent(S.days[d])).sort();
    return ds[ds.length - 1] || null;
  }
  function openCarry() {
    const from = prevDayWithContent(TODAY);
    if (!from) return false;
    const pd = S.days[from];
    const td = day(TODAY);
    const taken = new Set(td.todos.concat(td.blockers).flatMap((i) => [i.carriedFrom, i.splitFrom]).filter(Boolean));
    const rows = pd.todos.concat(pd.blockers)
      .filter((i) => i.status === 'open' && !taken.has(i.id))
      .map((i) => ({ item: i, checked: true, action: null, children: '' }));
    if (!rows.length) return false;
    if (S.date !== TODAY || S.mode !== 'board') switchTo(TODAY, 'board');
    closePop(false);
    S.carry = { from, rows, sel: 0 };
    S.ui.overlay = 'carry';
    setOverlayOpen(true);
    renderCarry(() => focusCarry(0));
    return true;
  }
  function renderCarry(after) {
    const L = M();
    const c = S.carry;
    const old = $('ovCarry').querySelector('.carry-body');
    const scroll = old ? old.scrollTop : 0;
    const row = (r, i) => {
      const it = r.item;
      const cls = 'crow' + (r.action === 'drop' ? ' dropped' : r.action === 'split' ? ' split' : (!r.checked ? ' off' : ''));
      const cb = h('input', { type: 'checkbox', class: 'cbox', tabindex: i === c.sel ? '0' : '-1', 'data-i': i, 'aria-label': it.text });
      cb.checked = r.checked && !r.action;
      const li = h('li', { class: cls, 'data-i': i },
        h('label', { class: 'cmain' },
          cb,
          it.origin === 'agent' ? h('span', { class: 'mk-agent', role: 'img', 'aria-label': L.agentItem, title: L.agentItem }) : null,
          h('span', { class: 'ctext' }, it.text),
          r.action ? h('span', { class: 'cstate' }, r.action === 'drop' ? L.dropState : L.splitState) : null,
          it.carry > 0 ? h('span', { class: 'carry' + (it.carry >= WARN_AT ? ' warn' : ''), role: 'img', 'aria-label': L.carryTimes(it.carry) }, carryMark(it.carry)) : null));
      if (it.kind === 'todo' && it.carry >= REVIEW_AT) {
        li.append(h('div', { class: 'review' },
          h('span', { class: 'msg' }, L.review(it.carry)),
          h('button', { type: 'button', class: 'btn', 'aria-pressed': String(r.action === 'split'), 'data-act': 'split', 'data-i': i }, L.split),
          h('button', { type: 'button', class: 'btn', 'aria-pressed': String(r.action === 'drop'), 'data-act': 'drop', 'data-i': i }, L.drop)));
        if (r.action === 'split') {
          const ta = h('textarea', { rows: Math.max(2, r.children.split('\n').length + 1), placeholder: L.splitPh, 'aria-label': L.splitPh, 'data-i': i });
          ta.value = r.children;
          li.append(h('div', { class: 'split-edit' }, ta));
        }
      }
      return li;
    };
    const idx = c.rows.map((r, i) => [r, i]);
    const todos = idx.filter(([r]) => r.item.kind === 'todo');
    const blks = idx.filter(([r]) => r.item.kind === 'blocker');
    const body = h('div', { class: 'carry-body' },
      todos.length ? [h('h3', { class: 'label' }, L.todo), h('ul', { class: 'clist' }, todos.map(([r, i]) => row(r, i)))] : null,
      blks.length ? [h('h3', { class: 'label' }, L.blockers), h('ul', { class: 'clist' }, blks.map(([r, i]) => row(r, i)))] : null);
    const panel = h('div', { class: 'panel carry-panel', role: 'dialog', 'aria-modal': 'true', 'aria-labelledby': 'carryTitle' },
      h('div', { class: 'ph' },
        h('div', { class: 'label' }, L.carryOver),
        h('h2', { id: 'carryTitle' }, L.carryFrom(dateLabel(c.from)))),
      body,
      h('div', { class: 'actions' },
        h('button', { type: 'button', class: 'btn primary', 'data-act': 'sel' }, L.carrySel, h('kbd', null, 'Enter')),
        h('button', { type: 'button', class: 'btn', 'data-act': 'all' }, L.carryAll),
        h('button', { type: 'button', class: 'btn', 'data-act': 'none' }, L.carryNone),
        h('button', { type: 'button', class: 'btn quiet later', 'data-act': 'later' }, L.carryLater, h('kbd', null, 'Esc'))));
    $('ovCarry').replaceChildren(panel);
    $('ovCarry').hidden = false;
    body.scrollTop = scroll;
    if (after) after();
  }
  function focusCarry(i) {
    const c = S.carry;
    i = Math.max(0, Math.min(c.rows.length - 1, i));
    c.sel = i;
    for (const cb of $('ovCarry').querySelectorAll('.cbox')) cb.tabIndex = +cb.dataset.i === i ? 0 : -1;
    const cb = $('ovCarry').querySelector(`.cbox[data-i="${i}"]`);
    if (cb) { cb.focus(); cb.closest('.crow').scrollIntoView({ block: 'nearest' }); }
  }
  function closeCarry() {
    S.carry = null;
    S.ui.overlay = null;
    $('ovCarry').hidden = true;
    $('ovCarry').replaceChildren();
    setOverlayOpen(false);
    renderLists();
    renderHeader();
    focusRegion('todo');
  }
  function applyCarry(mode) { // 'sel' 選んだもの / 'all' すべて / 'none' 持ち越さない
    const c = S.carry;
    const td = day(TODAY);
    const nt = [], nb = [];
    for (const r of c.rows) {
      const it = r.item;
      if (r.action === 'drop') { it.status = 'dropped'; it.closeReason = 'dropped'; continue; }
      if (r.action === 'split') {
        const kids = r.children.split('\n').map((s) => s.trim()).filter(Boolean);
        if (kids.length) {
          it.status = 'dropped';
          it.closeReason = 'split';
          for (const k of kids) nt.push(newItem('todo', k, { splitFrom: it.id })); // 子は今日に作り、持ち越し回数 0
          continue;
        }
      }
      const carry = mode === 'all' || (mode === 'sel' && (r.checked || r.action === 'split'));
      if (!carry) continue;
      const n = newItem(it.kind, it.text, { carry: it.carry + 1, carriedFrom: it.id, origin: it.origin });
      (it.kind === 'todo' ? nt : nb).push(n);
    }
    td.todos.unshift(...nt);
    td.blockers.unshift(...nb);
    td.carryDone = true;
    markDirty();
    closeCarry();
  }
  function resetCarry() { // 試作品：持ち越しを最初からやり直す
    if (S.ui.overlay === 'palette') closePalette(false);
    if (S.ui.overlay === 'carry') return;
    const td = day(TODAY);
    td.todos = td.todos.filter((i) => !i.carriedFrom && !i.splitFrom);
    td.blockers = td.blockers.filter((i) => !i.carriedFrom);
    td.carryDone = false;
    for (const d of Object.values(S.days)) for (const i of d.todos) if (i.closeReason) { i.status = 'open'; delete i.closeReason; }
    if (S.date === TODAY) renderLists();
    renderHeader();
    openCarry();
  }
  function carryKey(e) {
    const t = e.target;
    if (t.tagName === 'TEXTAREA') {
      if (e.key === 'Escape') { stop(e); focusCarry(+t.dataset.i); }
      return;
    }
    const plain = !e.ctrlKey && !e.metaKey && !e.altKey;
    if (e.key === 'Escape') { stop(e); closeCarry(); return; }
    if (e.key === 'Enter' && t.tagName !== 'BUTTON') { stop(e); applyCarry('sel'); return; }
    const i = t.dataset && t.dataset.i != null ? +t.dataset.i : S.carry.sel;
    if (plain && (e.key === 'j' || e.key === 'ArrowDown')) { stop(e); focusCarry(i + 1); return; }
    if (plain && (e.key === 'k' || e.key === 'ArrowUp')) { stop(e); focusCarry(i - 1); return; }
    if (shortcutOf(e) || undoKind(e)) stop(e); // 持ち越しの間はほかの操作を受け付けない
  }
  $('ovCarry').addEventListener('click', (e) => {
    const b = e.target.closest('button[data-act]');
    if (!b) return;
    const act = b.dataset.act;
    if (act === 'split' || act === 'drop') {
      const i = +b.dataset.i;
      const r = S.carry.rows[i];
      r.action = r.action === act ? null : act;
      r.checked = !r.action;
      S.carry.sel = i;
      renderCarry(() => {
        if (act === 'split' && r.action) $('ovCarry').querySelector(`textarea[data-i="${i}"]`).focus();
        else $('ovCarry').querySelector(`button[data-act="${act}"][data-i="${i}"]`).focus();
      });
      return;
    }
    if (act === 'later') { closeCarry(); return; }
    applyCarry(act);
  });
  $('ovCarry').addEventListener('change', (e) => {
    const cb = e.target;
    if (!cb.classList.contains('cbox')) return;
    const i = +cb.dataset.i;
    const r = S.carry.rows[i];
    r.checked = cb.checked;
    if (r.checked) r.action = null;
    renderCarry(() => focusCarry(i));
  });
  $('ovCarry').addEventListener('input', (e) => {
    const ta = e.target;
    if (ta.tagName !== 'TEXTAREA') return;
    S.carry.rows[+ta.dataset.i].children = ta.value;
    ta.rows = Math.max(2, ta.value.split('\n').length + 1);
  });
  $('ovCarry').addEventListener('focusin', (e) => {
    const t = e.target;
    if (t.classList && t.classList.contains('cbox')) {
      S.carry.sel = +t.dataset.i;
      for (const cb of $('ovCarry').querySelectorAll('.cbox')) cb.tabIndex = cb === t ? 0 : -1;
    }
  });

  /* ---------- S-04 検索パレット ---------- */

  function parseDateQuery(q) { // 03 6章：日付（2026-10-31、yesterday、昨日）は日付ジャンプ
    const s = q.trim().toLowerCase();
    if (!s) return null;
    const rel = { today: 0, '今日': 0, 'きょう': 0, yesterday: -1, '昨日': -1, 'きのう': -1, tomorrow: 1, '明日': 1, 'あした': 1, '一昨日': -2, 'おととい': -2 };
    if (s in rel) return addDays(TODAY, rel[s]);
    const valid = (y, m, d) => {
      const dt = new Date(Date.UTC(y, m - 1, d));
      return dt.getUTCFullYear() === y && dt.getUTCMonth() === m - 1 && dt.getUTCDate() === d ? fmtD(dt) : null;
    };
    let m = /^(\d{4})[-/.](\d{1,2})[-/.](\d{1,2})$/.exec(s);
    if (m) return valid(+m[1], +m[2], +m[3]);
    m = /^(\d{4})(\d{2})(\d{2})$/.exec(s);
    if (m) return valid(+m[1], +m[2], +m[3]);
    m = /^(\d{1,2})[-/](\d{1,2})$/.exec(s);
    if (m) return valid(+TODAY.slice(0, 4), +m[1], +m[2]);
    return null;
  }
  function commands() {
    const L = M();
    const cur = (b) => (b ? L.current : '');
    const c = [];
    c.push({ label: S.mode === 'board' ? L.cmdToNote : L.cmdToBoard, alias: 'mode note board モード ノート 整理', right: keyLabel('toggleMode'), run: toggleMode });
    if (S.date !== TODAY) c.push({ label: L.cmdToday, alias: 'today 今日', right: keyLabel('today'), run: () => goDate(TODAY) });
    c.push({ label: L.cmdPrev, alias: 'previous day 前日 昨日', right: keyLabel('prevDay'), run: () => goDate(addDays(S.date, -1)) });
    c.push({ label: L.cmdNext, alias: 'next day 翌日 明日', right: keyLabel('nextDay'), run: () => goDate(addDays(S.date, 1)) });
    if (!day(TODAY).carryDone && prevDayWithContent(TODAY)) c.push({ label: L.cmdCarry, alias: 'carry over 持ち越し', run: openCarry }); // 後回しにした持ち越しを開き直す
    c.push({ label: L.cmdArchive, alias: 'archive look back ふりかえり 振り返り 過去 週', right: keyLabel('archive'), run: openArchive });
    c.push({ label: L.cmdSidebar, alias: 'sidebar サイドバー', right: keyLabel('sidebar'), keep: true, run: toggleSidebar });
    for (const x of ['A', 'B', 'C']) c.push({ label: `${L.layout}${L.sep}${L['layout' + x]}`, alias: 'layout レイアウト', right: cur(S.set.layout === x), keep: true, run: () => setSetting('layout', x) });
    for (const [x, k] of [['system', 'themeSystem'], ['light', 'themeLight'], ['dark', 'themeDark']]) {
      c.push({ label: `${L.theme}${L.sep}${L[k]}`, alias: 'theme テーマ appearance 外観', right: cur(S.set.theme === x), keep: true, run: () => setSetting('theme', x) });
    }
    c.push({ label: L.langJa, alias: 'language 言語 japanese', right: cur(S.set.lang === 'ja'), keep: true, run: () => setSetting('lang', 'ja') });
    c.push({ label: L.langEn, alias: 'language 言語 english', right: cur(S.set.lang === 'en'), keep: true, run: () => setSetting('lang', 'en') });
    if (S.mode === 'board') {
      c.push({ label: L.cmdTemplate, alias: 'reflection template 振り返り テンプレート', right: keyLabel('refTemplate'), run: () => { focusRegion('reflection'); insertTemplate(); } });
      c.push({ label: L.cmdExport, alias: 'agent request export エージェント 依頼 書き出し', right: keyLabel('exportReq'), keep: true, run: () => exportRequest(S.palReturn) });
    }
    c.push({ label: L.cmdSettings, alias: 'settings preferences 設定', right: keyLabel('settings'), keep: true, run: () => flash(L.protoNoSettings) });
    c.push({ label: L.cmdHistory, alias: 'agent history エージェント 変更 履歴', keep: true, run: openHistory });
    if (S.ui.saveFail) c.push({ label: L.cmdSave, alias: 'save 保存', run: () => { S.popReturn = S.palReturn; openPop('save'); } });
    if (S.ui.update) c.push({ label: L.cmdUpdate, alias: 'update restart 更新 再起動', keep: true, run: () => flash(L.protoNoUpdate) });
    return c;
  }
  function search(q) { // 全日付の TODO・障害・振り返り・ノート（F-10）
    const ql = q.toLowerCase();
    const out = [];
    for (const date of Object.keys(S.days).sort().reverse()) {
      const d = S.days[date];
      for (const i of d.todos) if (i.status !== 'dropped' && i.text.toLowerCase().includes(ql)) out.push({ type: 'hit', date, kind: 'todo', text: i.text, id: i.id });
      for (const i of d.blockers) if (i.text.toLowerCase().includes(ql)) out.push({ type: 'hit', date, kind: 'blocker', text: i.text, id: i.id });
      d.reflection.forEach((t, line) => { if (t.toLowerCase().includes(ql)) out.push({ type: 'hit', date, kind: 'reflection', text: t, line }); });
      d.note.forEach((t, line) => { if (t.toLowerCase().includes(ql)) out.push({ type: 'hit', date, kind: 'note', text: t, line }); });
    }
    return out.slice(0, 40);
  }
  function palRows(q) {
    const L = M();
    const rows = [];
    const d = parseDateQuery(q);
    if (d) rows.push({ type: 'grp', label: L.grpDate }, { type: 'date', date: d });
    const ql = q.trim().toLowerCase();
    const cmds = commands().filter((c) => !ql || `${c.label} ${c.alias}`.toLowerCase().includes(ql));
    if (cmds.length) rows.push({ type: 'grp', label: L.grpCmd }, ...cmds.map((c) => ({ type: 'cmd', cmd: c })));
    if (ql) {
      const hits = search(q.trim());
      if (hits.length) rows.push({ type: 'grp', label: L.grpHits }, ...hits);
      else if (!d && !cmds.length) rows.push({ type: 'none' });
    }
    return rows;
  }
  function hlText(text, q) {
    const i = q ? text.toLowerCase().indexOf(q.toLowerCase()) : -1;
    if (i < 0) return [text];
    let start = 0;
    let pre = '';
    if (i > 28) { start = i - 12; pre = '…'; }
    return [pre + text.slice(start, i), h('b', null, text.slice(i, i + q.length)), text.slice(i + q.length)];
  }
  function openPalette() {
    closePop(false);
    S.palReturn = S.mode === 'board' ? (currentFocus() || S.focus[S.date]) : null;
    saveFocusState();
    S.pal = { q: '', sel: 0, items: [] };
    S.ui.overlay = 'palette';
    const L = M();
    const inp = h('input', { class: 'pal-in', type: 'text', placeholder: L.palPh, 'aria-label': L.palette, autocomplete: 'off', spellcheck: 'false', role: 'combobox', 'aria-expanded': 'true', 'aria-controls': 'palList' });
    const list = h('ul', { class: 'pal-list', id: 'palList', role: 'listbox', 'aria-label': L.palette });
    $('ovPal').replaceChildren(h('div', { class: 'panel pal-panel', role: 'dialog', 'aria-modal': 'true', 'aria-label': L.palette }, inp, list));
    $('ovPal').hidden = false;
    setOverlayOpen(true);
    inp.addEventListener('input', () => { S.pal.q = inp.value; S.pal.sel = 0; renderPalList(); });
    renderPalList();
    inp.focus();
  }
  function renderPalList() {
    const L = M();
    const q = S.pal.q.trim();
    const rows = palRows(S.pal.q);
    const list = $('palList');
    S.pal.items = [];
    const els = rows.map((r) => {
      if (r.type === 'grp') return h('li', { class: 'pal-grp label', role: 'presentation' }, r.label);
      if (r.type === 'none') return h('li', { class: 'pal-none', role: 'presentation' }, L.noHits);
      const n = S.pal.items.length;
      S.pal.items.push(r);
      const li = h('li', { class: 'pal-row', role: 'option', id: `pal-${n}`, 'data-n': n });
      if (r.type === 'date') {
        const rel = diffDays(r.date, TODAY);
        li.append(h('span', { class: 'pt' }, fullDate(r.date)), h('span', { class: 'pr' }, L.rel[rel] || ''));
      } else if (r.type === 'cmd') {
        li.append(...[h('span', { class: 'pt' }, r.cmd.label), r.cmd.right ? h('span', { class: 'pr' }, r.cmd.right) : null].filter(Boolean));
      } else {
        li.append(h('span', { class: 'pd' }, dateLabel(r.date)), h('span', { class: 'pk' }, L.kind[r.kind]), h('span', { class: 'pt' }, hlText(r.text, q)));
      }
      return li;
    });
    list.replaceChildren(...els);
    if (S.pal.sel >= S.pal.items.length) S.pal.sel = 0;
    renderPalSel();
  }
  function renderPalSel() {
    const list = $('palList');
    for (const li of list.querySelectorAll('.pal-row')) {
      const on = +li.dataset.n === S.pal.sel;
      li.classList.toggle('sel', on);
      li.setAttribute('aria-selected', String(on));
      if (on) li.scrollIntoView({ block: 'nearest' });
    }
    const inp = $('ovPal').querySelector('.pal-in');
    if (inp) inp.setAttribute('aria-activedescendant', S.pal.items.length ? `pal-${S.pal.sel}` : '');
  }
  function closePalette(restore) {
    S.ui.overlay = null;
    S.pal = null;
    $('ovPal').hidden = true;
    $('ovPal').replaceChildren();
    setOverlayOpen(false);
    if (restore) restoreAfterOverlay(S.palReturn);
  }
  function execPal(it) {
    if (it.type === 'date') { closePalette(false); goDate(it.date); return; }
    if (it.type === 'cmd') {
      closePalette(!!it.cmd.keep);
      it.cmd.run();
      return;
    }
    closePalette(false);
    const mode = it.kind === 'note' ? 'note' : 'board';
    switchTo(it.date, mode);
    if (it.kind === 'note') noteEd.focus({ line: it.line, offset: 0 });
    else if (it.kind === 'reflection') refEd.focus({ line: it.line, offset: 0 });
    else focusItem(it.id, 'end');
  }
  function palKey(e) {
    const sc = shortcutOf(e);
    if (e.key === 'Escape' || sc === 'palette') { stop(e); closePalette(true); return; }
    const n = S.pal.items.length;
    if (e.key === 'ArrowDown') { stop(e); if (n) { S.pal.sel = (S.pal.sel + 1) % n; renderPalSel(); } return; }
    if (e.key === 'ArrowUp') { stop(e); if (n) { S.pal.sel = (S.pal.sel - 1 + n) % n; renderPalSel(); } return; }
    if (e.key === 'Enter' && !e.shiftKey) { stop(e); const it = S.pal.items[S.pal.sel]; if (it) execPal(it); return; }
    if (sc) stop(e);
  }
  $('ovPal').addEventListener('mousedown', (e) => {
    if (e.target === $('ovPal')) { e.preventDefault(); closePalette(true); return; }
    const li = e.target.closest('.pal-row');
    if (li) { e.preventDefault(); execPal(S.pal.items[+li.dataset.n]); }
  });
  $('ovPal').addEventListener('mousemove', (e) => {
    const li = e.target.closest('.pal-row');
    if (li && S.pal && +li.dataset.n !== S.pal.sel) { S.pal.sel = +li.dataset.n; renderPalSel(); }
  });
  $('ovCarry').addEventListener('mousedown', (e) => { if (e.target === $('ovCarry')) e.preventDefault(); });

  /* ---------- キー入力の入口 ---------- */

  let justComposed = false;
  document.addEventListener('compositionend', () => { // 04 3章 2：WebKit 系の確定 Enter を無視するため
    justComposed = true;
    setTimeout(() => { justComposed = false; }, 0);
  }, true);

  function runShortcut(id) {
    switch (id) {
      case 'toggleMode': toggleMode(); break;
      case 'prevDay': if (S.mode === 'archive') archMove(-7); else goDate(addDays(S.date, -1)); break;
      case 'nextDay': if (S.mode === 'archive') archMove(7); else goDate(addDays(S.date, 1)); break;
      case 'sidebar': toggleSidebar(); break;
      case 'archive': if (S.mode === 'archive') leaveArchive(S.date); else openArchive(); break;
      case 'today': goDate(TODAY); break;
      case 'palette': openPalette(); break;
      case 'settings': flash(M().protoNoSettings); break;
      case 'vimEsc': vimEscape(); break;
      case 'region1': case 'region2': case 'region3':
        if (S.mode === 'board') focusRegion(['todo', 'blocker', 'reflection'][+id.slice(-1) - 1]);
        break;
      case 'refTemplate': if (S.mode === 'board') insertTemplate(); break;
      case 'sendTodo': if (S.mode === 'note') sendLines('todo'); break;
      case 'sendBlocker': if (S.mode === 'note') sendLines('blocker'); break;
      case 'timeHeading': if (S.mode === 'note') insertTimeHeading(); break;
      case 'exportReq': if (S.mode === 'board') exportRequest(); break;
      default: break;
    }
  }
  function doUndo(kind) {
    const a = document.activeElement;
    if (refEd.el.contains(a)) { if (kind === 'undo') refEd.undo(); else refEd.redo(); return; }
    if (noteEd.el.contains(a)) { if (kind === 'undo') noteEd.undo(); else noteEd.redo(); return; }
    if (S.mode === 'board') { if (kind === 'undo') boardUndo(); else boardRedo(); }
  }
  document.addEventListener('keydown', (e) => {
    if (e.isComposing || e.keyCode === 229) return; // 04 3章：変換中はショートカットとして扱わない
    if (justComposed && e.key === 'Enter') { justComposed = false; stop(e); return; }
    if (e.altKey && e.shiftKey && !e.ctrlKey && !e.metaKey && e.code === 'KeyP') { stop(e); toggleProto(); return; }
    if ($('proto').contains(document.activeElement)) { if (e.key === 'Escape') { stop(e); toggleProto(false); } return; }
    if (!$('ctx').hidden) {
      if (e.key === 'Escape') { stop(e); closeCtx(true); return; }
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        stop(e);
        const bs = [...$('ctx').querySelectorAll('button')];
        const i = bs.indexOf(document.activeElement);
        bs[(i + (e.key === 'ArrowDown' ? 1 : -1) + bs.length) % bs.length].focus();
      }
      return;
    }
    if (S.ui.overlay === 'carry') { carryKey(e); return; }
    if (S.ui.overlay === 'palette') { palKey(e); return; }
    if (S.ui.pop && e.key === 'Escape') { stop(e); closePop(true); return; }
    const sc = shortcutOf(e);
    if (sc) { stop(e); runShortcut(sc); return; }
    const uk = undoKind(e);
    if (uk) { stop(e); doUndo(uk); }
  }, true);

  /* ---------- 設定（試作品では S-05 の代わりに試作品の操作パネルから変える） ---------- */

  function setSetting(k, v) {
    if (k === 'fs') S.set.fs = +v;
    else if (k === 'noteMono' || k === 'vim') S.set[k] = v === 'on';
    else if (k === 'narrow') S.ui.narrow = v === 'on';
    else if (k === 'clock') S.ui.clock = v === 'real' ? null : +v;
    else S.set[k] = v;
    if (k === 'vim') {
      const a = document.activeElement;
      const li = a && a.closest && a.closest('#board .item');
      S.vim = li && a !== li ? 'insert' : 'normal';
      if (!S.set.vim && li && a === li) focusItem(li.dataset.id, 'end', true);
    }
    applySettings();
    renderStatic();
    renderHeader();
    if (k === 'lang') {
      const f = currentFocus();
      renderLists();
      if (S.ui.overlay === 'carry') renderCarry(() => focusCarry(S.carry.sel));
      else if (S.ui.overlay === 'palette') renderPalList();
      else if (f && f.id) restoreBoardFocus(f);
    }
    if (S.mode === 'archive') renderArchive();
    if (S.mode === 'note') renderOutline();
    renderVimInd();
    renderProto();
  }

  /* ---------- 試作品の操作パネル ---------- */

  const protoPanel = $('protoPanel');
  function toggleProto(force) {
    const open = force == null ? protoPanel.hidden : force;
    protoPanel.hidden = !open;
    $('protoBtn').setAttribute('aria-expanded', String(open));
    if (open) { renderProto(); (protoPanel.querySelector('button[aria-pressed="true"]') || protoPanel.querySelector('button')).focus(); }
    else if (!S.ui.overlay) restoreAfterOverlay(S.focus[S.date]);
  }
  function renderProto() {
    const on = {
      layout: S.set.layout, theme: S.set.theme, lang: S.set.lang, fs: String(S.set.fs),
      noteMono: S.set.noteMono ? 'on' : 'off', vim: S.set.vim ? 'on' : 'off', narrow: S.ui.narrow ? 'on' : 'off',
      clock: S.ui.clock == null ? 'real' : String(S.ui.clock),
    };
    for (const b of protoPanel.querySelectorAll('button[data-set]')) {
      const [k, v] = b.dataset.set.split(':');
      b.setAttribute('aria-pressed', String(on[k] === v));
    }
    const tg = { fail: S.ui.failMode, hold: S.ui.holdSaving, update: S.ui.update };
    for (const b of protoPanel.querySelectorAll('button[data-toggle]')) b.setAttribute('aria-pressed', String(!!tg[b.dataset.toggle]));
    renderProtoStatus();
  }
  function renderProtoStatus() {
    const el = $('protoStatus');
    if (!el) return;
    const names = { A: '2段', B: '3列', C: '縦積み' };
    const eff = effLayout();
    const osDark = matchMedia('(prefers-color-scheme: dark)').matches;
    const theme = S.set.theme === 'system' ? `OS に合わせる（いまの OS：${osDark ? 'ダーク' : 'ライト'}）` : (S.set.theme === 'light' ? 'ライト' : 'ダーク');
    const why = eff !== S.set.layout ? `（幅 ${Math.round(S.ui.width)}px で 900px 未満のため。設定は ${names[S.set.layout]}）` : '';
    el.textContent = `表示中のレイアウト：${names[eff]}${why}／テーマ：${theme}／凪メーター：${S.ui.clock == null ? 'いまの時刻' : hhmm(S.ui.clock)}`;
  }
  $('proto').addEventListener('click', (e) => {
    const b = e.target.closest('button');
    if (!b) return;
    if (b.id === 'protoBtn') { toggleProto(); return; }
    if (b.dataset.set) { const [k, v] = b.dataset.set.split(':'); setSetting(k, v); return; }
    if (b.dataset.toggle) {
      const t = b.dataset.toggle;
      if (t === 'fail') { S.ui.failMode = !S.ui.failMode; doSave(); }
      if (t === 'hold') { S.ui.holdSaving = !S.ui.holdSaving; renderStatus(); }
      if (t === 'update') { S.ui.update = !S.ui.update; renderHeader(); }
      renderProto();
      return;
    }
    const act = b.dataset.act;
    if (act === 'carry') { toggleProto(false); resetCarry(); }
    if (act === 'palette') { toggleProto(false); if (S.ui.overlay !== 'palette') { if (S.ui.overlay === 'carry') closeCarry(); openPalette(); } }
    if (act === 'mode') { toggleProto(false); if (!S.ui.overlay) toggleMode(); }
    if (act === 'empty') { toggleProto(false); if (!S.ui.overlay) goDate(addDays(TODAY, 1)); }
    if (act === 'prev') { toggleProto(false); if (!S.ui.overlay) goDate(addDays(TODAY, -1)); }
    if (act === 'archive') { toggleProto(false); if (!S.ui.overlay) openArchive(); }
    if (act === 'unseen') {
      for (const id of ['t4', 'b1']) { const it = findItem(id, TODAY); if (it) it.unseen = true; }
      renderLists(); renderHeader();
    }
  });
  matchMedia('(prefers-color-scheme: dark)').addEventListener('change', renderProtoStatus);

  /* ---------- 起動 ---------- */

  new ResizeObserver(([en]) => { S.ui.width = en.contentRect.width; applyLayout(); }).observe($('app'));
  renderStatic();
  applySettings();
  loadDay();
  applyMode();
  renderProto();
  if (day(TODAY).carryDone || !openCarry()) focusRegion('todo'); // 03 9章：その日の初回表示なら S-03

  window.__yunagi = { S, goDate, toggleMode, openPalette, resetCarry, setSetting, openArchive, toggleSidebar };
})();
