// CodeMirror 6 のエディタを1つ作る。Svelte 版・Solid 版で同じファイルを使う。
import { EditorView, basicSetup } from 'codemirror';
import { markdown } from '@codemirror/lang-markdown';

const theme = EditorView.theme({
  '&': {
    color: 'var(--text)',
    backgroundColor: 'var(--surface)',
    fontSize: '15px',
    height: '100%',
  },
  '.cm-content': { lineHeight: '1.95', caretColor: 'var(--accent-strong)' },
  '.cm-gutters': {
    backgroundColor: 'var(--surface)',
    color: 'var(--text-muted)',
    borderRight: '1px solid var(--rule)',
  },
  '&.cm-focused': { outline: '2px solid var(--accent-strong)' },
  '.cm-activeLine': { backgroundColor: 'var(--sel)' },
  '.cm-activeLineGutter': { backgroundColor: 'var(--sel)' },
});

const initialDoc = `## 10:30 調査メモ
- ここは CodeMirror 6 のエディタです
- 日本語を入力して、変換・確定・Enter の挙動を確かめてください
`;

export function createEditor(parent: HTMLElement): EditorView {
  const view = new EditorView({
    doc: initialDoc,
    extensions: [basicSetup, markdown(), EditorView.lineWrapping, theme],
    parent,
  });
  // 計測コード（bench.ts）から触れるようにする
  (window as unknown as { __cmView: EditorView }).__cmView = view;
  return view;
}
