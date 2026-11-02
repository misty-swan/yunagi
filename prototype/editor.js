/*
  行エディタ（ノート・振り返り共通）
  試作品用の簡易な実装。本番は CodeMirror 6 を使う（01 アーキテクチャ設計書）。

  - 1行 = <div class="ln">。「## 」で始まる行は見出し。「## 」は薄く、続く時刻（10:00 など）は大きく表示する
  - 送信済みの印は markFor(行の本文) が返す種類（'todo' / 'blocker'）で付ける。
    本文が変わって一致しなくなれば印は外れる（02 データモデル note_send）
  - 改行・行をまたぐ削除・貼り付け・取り消しはこのクラスで処理する（ブラウザ任せにすると行の構造が崩れるため）
  - 日本語入力の変換中（composition 中）は何もしない（04 操作仕様書 3章）
*/
(function () {
  const isMac = /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);
  const HEAD = /^(#{1,3} )/;
  const TIME = /^(\d{1,2}:\d{2})/;

  class LineEditor {
    constructor(el, opts = {}) {
      this.el = el;
      this.onChange = opts.onChange || (() => {});
      this.markFor = opts.markFor || (() => null);
      this.u = [];
      this.r = [];
      this.lastKind = null;
      this.lastLine = -1;
      this.composing = false;
      el.classList.add('le');
      el.contentEditable = 'true';
      el.spellcheck = false;
      el.setAttribute('role', 'textbox');
      el.setAttribute('aria-multiline', 'true');
      el.addEventListener('beforeinput', (e) => this.onBeforeInput(e));
      el.addEventListener('input', (e) => this.onInput(e));
      el.addEventListener('keydown', (e) => this.onKeyDown(e));
      el.addEventListener('paste', (e) => this.onPaste(e));
      el.addEventListener('copy', (e) => this.onCopy(e, false));
      el.addEventListener('cut', (e) => this.onCopy(e, true));
      el.addEventListener('dragstart', (e) => e.preventDefault());
      el.addEventListener('drop', (e) => e.preventDefault());
      el.addEventListener('mousedown', () => { this.lastKind = null; });
      el.addEventListener('compositionstart', () => { this.checkpoint('ime'); this.composing = true; });
      el.addEventListener('compositionend', () => {
        this.composing = false;
        setTimeout(() => { this.normalize(); this.emit(); }, 0);
      });
      this.setText(['']);
    }

    /* ---- 内容の読み書き ---- */

    setText(lines) {
      this.render(lines && lines.length ? lines : ['']);
      this.u = [];
      this.r = [];
      this.lastKind = null;
    }
    texts() { return [...this.el.childNodes].map((d) => d.textContent); }

    render(lines) {
      const frag = document.createDocumentFragment();
      for (const t of lines) frag.append(this.lineEl(t));
      this.el.replaceChildren(frag);
      if (!this.el.childNodes.length) this.el.append(this.lineEl(''));
    }
    lineEl(text) {
      const d = document.createElement('div');
      d.className = 'ln';
      this.fill(d, text);
      return d;
    }
    /* 行の中身の形：[種類, 文字] の並び。'md' = 「## 」、'tm' = 見出しの時刻、'#' = ふつうの文字 */
    expected(text) {
      const m = HEAD.exec(text);
      if (!m) return text === '' ? [] : [['#', text]];
      const out = [['md', m[1]]];
      let rest = text.slice(m[1].length);
      const t = TIME.exec(rest);
      if (t) { out.push(['tm', t[1]]); rest = rest.slice(t[1].length); }
      if (rest) out.push(['#', rest]);
      return out;
    }
    fill(d, text) {
      d.className = HEAD.test(text) ? 'ln h' : 'ln';
      this.setMark(d, text);
      const exp = this.expected(text);
      if (!exp.length) { d.replaceChildren(document.createElement('br')); return; }
      d.replaceChildren(...exp.map(([k, v]) => {
        if (k === '#') return document.createTextNode(v);
        const sp = document.createElement('span');
        sp.className = k;
        sp.textContent = v;
        return sp;
      }));
    }
    setMark(d, text) {
      const mark = text.trim() ? this.markFor(text) : null;
      if (mark) { if (d.dataset.sent !== mark) d.dataset.sent = mark; }
      else if (d.dataset.sent) delete d.dataset.sent;
    }
    refreshMarks() { for (const d of this.el.childNodes) if (d.nodeType === 1) this.setMark(d, d.textContent); }

    /* 行の DOM が期待どおりの形か */
    ok(d, text) {
      if (d.nodeType !== 1 || d.nodeName !== 'DIV') return false;
      if (d.classList.contains('h') !== HEAD.test(text)) return false;
      const exp = this.expected(text);
      let ch = [...d.childNodes];
      if (!exp.length) return ch.length === 0 || (ch.length === 1 && ch[0].nodeName === 'BR');
      if (ch.length === exp.length + 1 && ch[ch.length - 1].nodeName === 'BR' && exp[exp.length - 1][0] !== '#') ch = ch.slice(0, -1);
      if (ch.length !== exp.length) return false;
      return exp.every(([k, v], i) => {
        const n = ch[i];
        if (k === '#') return n.nodeType === 3 && n.data === v;
        return n.nodeName === 'SPAN' && n.className === k && n.childNodes.length === 1 && n.textContent === v;
      });
    }

    /* ブラウザの編集で崩れた構造を直す。カーソル位置は文字数で保つ */
    normalize() {
      const kids = [...this.el.childNodes];
      const broken = !kids.length || kids.some((n) => n.nodeType !== 1 || n.nodeName !== 'DIV' || n.querySelector('div,p'));
      const caret = this.getCaret();
      if (broken) {
        this.render(this.readLoose());
        if (caret) this.setCaret(caret);
        return;
      }
      kids.forEach((d, i) => {
        const text = d.textContent;
        if (!this.ok(d, text)) {
          this.fill(d, text);
          if (caret && caret.line === i) this.setCaret(caret);
        } else {
          this.setMark(d, text);
        }
      });
    }
    readLoose() {
      const out = [];
      let cur = null;
      const flush = () => { if (cur !== null) { out.push(cur); cur = null; } };
      const walk = (n, block) => {
        for (const c of n.childNodes) {
          if (c.nodeType === 3) { cur = (cur ?? '') + c.data; }
          else if (c.nodeName === 'BR') { out.push(cur ?? ''); cur = null; }
          else if (c.nodeName === 'DIV' || c.nodeName === 'P') { flush(); walk(c, true); flush(); }
          else walk(c, block);
        }
      };
      walk(this.el, true);
      flush();
      return out.length ? out : [''];
    }

    /* ---- カーソル ---- */

    pos(node, off) {
      if (node === this.el) {
        const n = this.el.childNodes.length;
        if (off >= n) return { line: n - 1, offset: (this.el.childNodes[n - 1] || { textContent: '' }).textContent.length };
        return { line: off, offset: 0 };
      }
      let n = node;
      while (n && n.parentNode !== this.el) n = n.parentNode;
      if (!n) return null;
      const line = [...this.el.childNodes].indexOf(n);
      const r = document.createRange();
      r.setStart(n, 0);
      r.setEnd(node, off);
      return { line, offset: r.toString().length };
    }
    getSel() {
      const s = getSelection();
      if (!s.rangeCount) return null;
      const r = s.getRangeAt(0);
      if (!this.el.contains(r.startContainer) || !this.el.contains(r.endContainer)) return null;
      const a = this.pos(r.startContainer, r.startOffset);
      const b = this.pos(r.endContainer, r.endOffset);
      return a && b ? { start: a, end: b, collapsed: r.collapsed } : null;
    }
    getCaret() { const s = this.getSel(); return s ? s.start : null; }
    point(p) {
      const kids = this.el.childNodes;
      const d = kids[Math.max(0, Math.min(p.line, kids.length - 1))];
      if (!d) return [this.el, 0];
      const w = document.createTreeWalker(d, NodeFilter.SHOW_TEXT);
      const ts = [];
      let t;
      while ((t = w.nextNode())) ts.push(t);
      let rem = Math.max(0, p.offset);
      for (let i = 0; i < ts.length; i++) {
        const last = i === ts.length - 1;
        if (rem < ts[i].length || (rem === ts[i].length && last)) return [ts[i], rem];
        rem -= ts[i].length;
      }
      return ts.length ? [ts[ts.length - 1], ts[ts.length - 1].length] : [d, 0];
    }
    setCaret(p) { this.setSel(p, p); }
    setSel(a, b) {
      const [n1, o1] = this.point(a);
      const [n2, o2] = this.point(b);
      const r = document.createRange();
      r.setStart(n1, o1);
      r.setEnd(n2, o2);
      const s = getSelection();
      s.removeAllRanges();
      s.addRange(r);
    }
    focus(p) {
      this.el.focus({ preventScroll: true });
      const lines = this.texts();
      const q = p || { line: lines.length - 1, offset: lines[lines.length - 1].length };
      this.setCaret(q);
      this.scrollToLine(q.line);
    }
    scrollToLine(i) {
      const d = this.el.childNodes[i];
      if (d && d.scrollIntoView) d.scrollIntoView({ block: 'nearest' });
    }
    selectedLineRange() {
      const s = this.getSel();
      if (!s) return null;
      let b = s.end.line;
      if (!s.collapsed && s.end.offset === 0 && b > s.start.line) b -= 1;
      return [s.start.line, b];
    }

    /* ---- 編集 ---- */

    replace(l1, o1, l2, o2, text) {
      const lines = this.texts();
      const before = lines[l1].slice(0, o1);
      const after = lines[l2].slice(o2);
      const parts = text.split('\n');
      const nl = parts.map((p, i) => (i === 0 ? before : '') + p + (i === parts.length - 1 ? after : ''));
      lines.splice(l1, l2 - l1 + 1, ...nl);
      this.render(lines);
      const last = parts.length - 1;
      const caret = { line: l1 + last, offset: (last === 0 ? before.length : 0) + parts[last].length };
      this.setCaret(caret);
      this.scrollToLine(caret.line);
      this.emit();
    }
    enter() {
      const s = this.getSel();
      if (!s) return;
      const lines = this.texts();
      let ins = '\n';
      if (s.collapsed) {
        const cur = lines[s.start.line];
        const m = /^(\s*)(- )(.*)$/.exec(cur);
        if (m) {
          if (m[3] === '' && s.start.offset === cur.length) {
            this.checkpoint('struct');
            this.replace(s.start.line, 0, s.start.line, cur.length, '');
            return;
          }
          if (s.start.offset >= m[1].length + m[2].length) ins = '\n' + m[1] + m[2];
        }
      }
      this.checkpoint('struct');
      this.replace(s.start.line, s.start.offset, s.end.line, s.end.offset, ins);
    }
    /* 現在行が空ならそこに、そうでなければ直後に行を挿入する */
    insertLinesAtCaret(newLines) {
      const lines = this.texts();
      const s = this.getSel();
      const L = s ? s.end.line : lines.length - 1;
      this.checkpoint('struct');
      if (lines[L].trim() === '') this.replace(L, 0, L, lines[L].length, newLines.join('\n'));
      else this.replace(L, lines[L].length, L, lines[L].length, '\n' + newLines.join('\n'));
    }

    /* ---- 取り消し（ブラウザ標準ではなく、行単位のスナップショット） ---- */

    checkpoint(kind, line) {
      if ((kind === 'type' || kind === 'del') && this.lastKind === kind && this.lastLine === line) return;
      this.u.push({ lines: this.texts(), caret: this.getCaret() });
      if (this.u.length > 300) this.u.shift();
      this.r = [];
      this.lastKind = kind;
      this.lastLine = line;
    }
    undo() {
      if (!this.u.length) return;
      this.r.push({ lines: this.texts(), caret: this.getCaret() });
      this.restore(this.u.pop());
    }
    redo() {
      if (!this.r.length) return;
      this.u.push({ lines: this.texts(), caret: this.getCaret() });
      this.restore(this.r.pop());
    }
    restore(snap) {
      this.render(snap.lines);
      if (snap.caret) { this.setCaret(snap.caret); this.scrollToLine(snap.caret.line); }
      this.lastKind = null;
      this.emit();
    }

    /* ---- イベント ---- */

    emit() { this.onChange(this.texts()); }

    onKeyDown(e) {
      if (e.isComposing || e.keyCode === 229 || this.composing || e.defaultPrevented) return;
      const mod = isMac ? e.metaKey : e.ctrlKey;
      if (e.key === 'Enter' && !mod && !e.altKey) { e.preventDefault(); this.enter(); return; }
      if (/^(Arrow|Home|End|Page)/.test(e.key)) this.lastKind = null;
    }
    onBeforeInput(e) {
      const t = e.inputType || '';
      if (t === 'historyUndo') { e.preventDefault(); this.undo(); return; }
      if (t === 'historyRedo') { e.preventDefault(); this.redo(); return; }
      if (t.startsWith('format')) { e.preventDefault(); return; }
      if (this.composing || e.isComposing || t === 'insertCompositionText') return;
      const s = this.getSel();
      if (!s) return;
      const multi = s.start.line !== s.end.line;
      if (t === 'insertParagraph' || t === 'insertLineBreak') { e.preventDefault(); this.enter(); return; }
      if (t === 'insertFromPaste' || t === 'insertFromDrop' || t === 'deleteByDrag' || t === 'deleteByCut' || t === 'insertFromYank') { e.preventDefault(); return; }
      if (t === 'insertText' || t === 'insertReplacementText') {
        if (multi) {
          e.preventDefault();
          this.checkpoint('struct');
          const data = e.data ?? (e.dataTransfer ? e.dataTransfer.getData('text/plain') : '');
          this.replace(s.start.line, s.start.offset, s.end.line, s.end.offset, data || '');
          return;
        }
        this.checkpoint('type', s.start.line);
        return;
      }
      if (t.startsWith('delete')) {
        if (multi) {
          e.preventDefault();
          this.checkpoint('struct');
          this.replace(s.start.line, s.start.offset, s.end.line, s.end.offset, '');
          return;
        }
        if (s.collapsed) {
          const back = t.includes('Backward');
          const lines = this.texts();
          if (back && s.start.offset === 0 && s.start.line > 0) {
            e.preventDefault();
            this.checkpoint('struct');
            const pl = s.start.line - 1;
            this.replace(pl, lines[pl].length, s.start.line, 0, '');
            return;
          }
          if (!back && s.start.offset === lines[s.start.line].length && s.start.line < lines.length - 1) {
            e.preventDefault();
            this.checkpoint('struct');
            this.replace(s.start.line, s.start.offset, s.start.line + 1, 0, '');
            return;
          }
        }
        this.checkpoint('del', s.start.line);
      }
    }
    onInput(e) {
      if (this.composing || e.isComposing) return;
      this.normalize();
      this.emit();
    }
    onPaste(e) {
      e.preventDefault();
      const text = (e.clipboardData.getData('text/plain') || '').replace(/\r\n?/g, '\n');
      const s = this.getSel();
      if (!s || !text) return;
      this.checkpoint('struct');
      this.replace(s.start.line, s.start.offset, s.end.line, s.end.offset, text);
    }
    onCopy(e, cut) {
      const s = this.getSel();
      if (!s || s.collapsed) return;
      e.preventDefault();
      const lines = this.texts();
      let txt;
      if (s.start.line === s.end.line) txt = lines[s.start.line].slice(s.start.offset, s.end.offset);
      else txt = [lines[s.start.line].slice(s.start.offset), ...lines.slice(s.start.line + 1, s.end.line), lines[s.end.line].slice(0, s.end.offset)].join('\n');
      e.clipboardData.setData('text/plain', txt);
      if (cut) {
        this.checkpoint('struct');
        this.replace(s.start.line, s.start.offset, s.end.line, s.end.offset, '');
      }
    }
  }

  window.LineEditor = LineEditor;
})();
