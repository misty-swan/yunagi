#!/usr/bin/env python3
"""工夫を省いた書き方（変種）を一時的に当ててビルドし、1回だけ listcheck を走らせ、元に戻す。

使い方（experiments/yun-2-frontend で）: python3 scripts/variants.py
結果は results/raw/variants.json。終わったら実行ファイルが最後の変種のままなので、
両版を pnpm tauri build し直すこと。
"""
import json, subprocess, sys
from pathlib import Path
ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / 'scripts'))
import measure

VARIANTS = {
  'solid-A-no-reconcile': ('solid', 'solid/src/ItemList.tsx',
      "setItems(reconcile(core.load(), { key: 'id' }));", "setItems(core.load());"),
  'solid-B-textContent-prop': ('solid', 'solid/src/ItemList.tsx',
      "            createEffect(() => {\n              if (el.textContent !== item.text) el.textContent = item.text;\n            });\n",
      "", ('                  ref={el}\n', '                  ref={el}\n                  textContent={item.text}\n')),
  'svelte-C-unkeyed-each': ('svelte', 'svelte/src/ItemList.svelte',
      "{#each items as item, i (item.id)}", "{#each items as item, i}"),
  'svelte-D-text-expression': ('svelte', 'svelte/src/ItemList.svelte',
      "        bind:textContent={item.text}\n        oninput={save}\n",
      "        oninput={(e) => { item.text = e.currentTarget.textContent ?? ''; save(); }}\n",
      ('      ></span>', '      >{item.text}</span>')),
  'svelte-E-no-tick': ('svelte', 'svelte/src/ItemList.svelte',
      "    await tick();\n", ""),
}
results = {}
for name, (app, rel, old, new, *extra) in VARIANTS.items():
    p = ROOT / rel
    orig = p.read_text()
    s = orig
    assert old in s, name
    s = s.replace(old, new)
    for o, n in extra:
        assert o in s, (name, o)
        s = s.replace(o, n)
    p.write_text(s)
    try:
        b = subprocess.run(['mise', 'exec', 'rust', '--', 'pnpm', 'tauri', 'build', '--no-bundle'], cwd=ROOT / app, capture_output=True, text=True)
        if b.returncode != 0:
            results[name] = {'build_error': (b.stdout + b.stderr)[-2000:]}
            continue
        r = measure.checked_run(app, 'listcheck')
        results[name] = r.get('bench')
    finally:
        p.write_text(orig)
    print(name, json.dumps(results[name], ensure_ascii=False)[:1500], flush=True)
(ROOT / 'results/raw/variants.json').write_text(json.dumps(results, ensure_ascii=False, indent=2))
