//! JavaScript that Pantry evaluates in Spotify's xpui page over CDP. Every snippet resolves to a JSON string.
//!
//! IndexedDB values go through a tagged encoding so non-JSON values survive a round trip:
//! `{"$t":"date"|"ab"|"ta"|"blob"|"map"|"set"|"re"|"undef"|"num"|"obj", ...}`. Plain JSON passes unchanged.
//! Reading never creates a database: only names from `indexedDB.databases()` are opened, without a version.

const CODEC: &str = r#"
const b64 = (buf) => { const u = new Uint8Array(buf); let s = ''; for (let i = 0; i < u.length; i += 0x8000) s += String.fromCharCode.apply(null, u.subarray(i, i + 0x8000)); return btoa(s); };
const unb64 = (s) => { const b = atob(s); const u = new Uint8Array(b.length); for (let i = 0; i < b.length; i++) u[i] = b.charCodeAt(i); return u; };
const enc = async (x) => {
  if (x === undefined) return { $t: 'undef' };
  if (x === null || typeof x === 'boolean' || typeof x === 'string') return x;
  if (typeof x === 'number') return Number.isFinite(x) ? x : { $t: 'num', v: String(x) };
  if (typeof x === 'bigint') return { $t: 'num', v: String(x), big: true };
  if (Array.isArray(x)) { const o = []; for (const y of x) o.push(await enc(y)); return o; }
  if (x instanceof Date) return { $t: 'date', v: x.getTime() };
  if (x instanceof ArrayBuffer) return { $t: 'ab', v: b64(x) };
  if (ArrayBuffer.isView(x)) return { $t: 'ta', c: x.constructor.name, v: b64(x.buffer.slice(x.byteOffset, x.byteOffset + x.byteLength)) };
  if (typeof Blob !== 'undefined' && x instanceof Blob) return { $t: 'blob', type: x.type, name: x.name, v: b64(await x.arrayBuffer()) };
  if (x instanceof Map) { const o = []; for (const [k, v] of x) o.push([await enc(k), await enc(v)]); return { $t: 'map', v: o }; }
  if (x instanceof Set) { const o = []; for (const v of x) o.push(await enc(v)); return { $t: 'set', v: o }; }
  if (x instanceof RegExp) return { $t: 're', v: x.source, f: x.flags };
  const o = {}; for (const k of Object.keys(x)) o[k] = await enc(x[k]);
  return Object.prototype.hasOwnProperty.call(x, '$t') ? { $t: 'obj', v: o } : o;
};
const dec = (x) => {
  if (x === null || typeof x !== 'object') return x;
  if (Array.isArray(x)) return x.map(dec);
  switch (x.$t) {
    case undefined: { const o = {}; for (const k of Object.keys(x)) o[k] = dec(x[k]); return o; }
    case 'undef': return undefined;
    case 'num': return x.big ? BigInt(x.v) : Number(x.v);
    case 'date': return new Date(x.v);
    case 'ab': return unb64(x.v).buffer;
    case 'ta': { const u = unb64(x.v); const C = globalThis[x.c] || Uint8Array; return C === DataView ? new DataView(u.buffer) : new C(u.buffer); }
    case 'blob': return x.name !== undefined && typeof File !== 'undefined' ? new File([unb64(x.v)], x.name, { type: x.type }) : new Blob([unb64(x.v)], { type: x.type });
    case 'map': return new Map(x.v.map(([k, v]) => [dec(k), dec(v)]));
    case 'set': return new Set(x.v.map(dec));
    case 're': return new RegExp(x.v, x.f);
    case 'obj': { const o = {}; for (const k of Object.keys(x.v)) o[k] = dec(x.v[k]); return o; }
    default: return x;
  }
};
const req = (r) => new Promise((ok, bad) => { r.onsuccess = () => ok(r.result); r.onerror = () => bad(r.error); });
"#;

/// Is Spicetify up in the page?
pub const READY: &str = "JSON.stringify(!!(window.Spicetify && Spicetify.Platform && Spicetify.LocalStorage && Spicetify.Config))";

/// Has Marketplace finished hydrating its IndexedDB? `null` when Marketplace doesn't use IndexedDB.
pub const MARKETPLACE_STATE: &str = r#"(async () => {
  const dbs = indexedDB.databases ? await indexedDB.databases() : [];
  const has = dbs.some(d => d.name === 'spicetify-marketplace');
  const legacy = Object.keys(localStorage).some(k => k.startsWith('marketplace:'));
  return JSON.stringify({ indexedDb: has, legacyKeys: legacy });
})()"#;

/// Everything a capture needs from the live page.
pub fn capture() -> String {
    format!(
        r#"(async () => {{ {CODEC}
  const ls = {{}};
  for (const k of Object.keys(localStorage).sort()) ls[k] = localStorage.getItem(k);
  const idb = {{}};
  for (const {{ name, version }} of (indexedDB.databases ? await indexedDB.databases() : [])) {{
    const db = await req(indexedDB.open(name));
    const stores = {{}};
    for (const s of db.objectStoreNames) {{
      const st = db.transaction(s, 'readonly').objectStore(s);
      const indexes = [...st.indexNames].map(n => {{ const i = st.index(n); return {{ name: n, keyPath: i.keyPath, unique: i.unique, multiEntry: i.multiEntry }}; }});
      const records = [];
      if (name !== 'keyval-store') {{
        const raw = await new Promise((ok, bad) => {{
          const out = []; const c = st.openCursor();
          c.onsuccess = () => {{ const cur = c.result; if (!cur) return ok(out); out.push([cur.key, cur.value]); cur.continue(); }};
          c.onerror = () => bad(c.error);
        }});
        for (const [k, v] of raw) records.push({{ key: await enc(k), value: await enc(v) }});
      }}
      stores[s] = {{ keyPath: st.keyPath, autoIncrement: st.autoIncrement, indexes, records }};
    }}
    db.close();
    idb[name] = {{ version, stores }};
  }}
  return JSON.stringify({{
    spotifyVersion: Spicetify.Platform && Spicetify.Platform.version,
    username: (Spicetify.Platform && Spicetify.Platform.username) || null,
    config: Spicetify.Config,
    scripts: [...document.scripts].map(s => s.src).filter(Boolean),
    localStorage: ls,
    indexedDB: idb
  }});
}})()"#
    )
}

/// Writes localStorage keys and IndexedDB records. `payload` is
/// `{ localStorage: {k: v}, indexedDB: { name: { version, stores: { s: { keyPath, autoIncrement, indexes, records } } } } }`.
/// Missing databases and stores are created from the captured schema; existing records with the same key are replaced.
/// `replace: [[db, store]]` also deletes records missing from the payload (except `internal:*` keys), and
/// `replaceLocalStoragePrefix` deletes localStorage keys with that prefix that the payload doesn't set.
pub fn restore(payload: &serde_json::Value) -> String {
    format!(
        r#"(async () => {{ {CODEC}
  const p = {payload};
  let keys = 0, records = 0; const errors = [];
  let removed = 0;
  if (p.replaceLocalStoragePrefix) for (const k of Object.keys(localStorage)) if (k.startsWith(p.replaceLocalStoragePrefix) && !(k in (p.localStorage || {{}}))) {{ localStorage.removeItem(k); removed++; }}
  for (const [k, v] of Object.entries(p.localStorage || {{}})) {{ try {{ localStorage.setItem(k, v); keys++; }} catch (e) {{ errors.push(k + ': ' + e); }} }}
  const existing = new Map((indexedDB.databases ? await indexedDB.databases() : []).map(d => [d.name, d.version]));
  for (const [name, d] of Object.entries(p.indexedDB || {{}})) {{
    try {{
      let db = existing.has(name) ? await req(indexedDB.open(name)) : null;
      const missing = Object.keys(d.stores).filter(s => !db || !db.objectStoreNames.contains(s));
      if (missing.length) {{
        const v = db ? db.version + 1 : Math.max(1, d.version || 1);
        if (db) db.close();
        db = await new Promise((ok, bad) => {{
          const r = indexedDB.open(name, v);
          r.onupgradeneeded = () => {{
            for (const s of missing) {{
              const sd = d.stores[s];
              const st = r.result.createObjectStore(s, {{ keyPath: sd.keyPath === null ? undefined : sd.keyPath, autoIncrement: !!sd.autoIncrement }});
              for (const i of sd.indexes || []) st.createIndex(i.name, i.keyPath, {{ unique: !!i.unique, multiEntry: !!i.multiEntry }});
            }}
          }};
          r.onsuccess = () => ok(r.result); r.onerror = () => bad(r.error);
          r.onblocked = () => bad(new Error('database ' + name + ' is in use and blocked the upgrade'));
        }});
      }}
      for (const [s, sd] of Object.entries(d.stores)) {{
        const replace = (p.replace || []).some(([a, b]) => a === name && b === s);
        if (!sd.records.length && !replace) continue;
        const tx = db.transaction(s, 'readwrite'); const st = tx.objectStore(s);
        if (replace) {{
          const keep = new Set(sd.records.map(r => JSON.stringify(dec(r.key))));
          const all = await new Promise((ok, bad) => {{ const r = st.getAllKeys(); r.onsuccess = () => ok(r.result); r.onerror = () => bad(r.error); }});
          for (const k of all) if (!keep.has(JSON.stringify(k)) && !(typeof k === 'string' && k.startsWith('internal:'))) {{ st.delete(k); removed++; }}
        }}
        for (const r of sd.records) {{ const v = dec(r.value); if (st.keyPath === null) st.put(v, dec(r.key)); else st.put(v); records++; }}
        await new Promise((ok, bad) => {{ tx.oncomplete = ok; tx.onerror = () => bad(tx.error); tx.onabort = () => bad(tx.error); }});
      }}
      db.close();
    }} catch (e) {{ errors.push(name + ': ' + e); }}
  }}
  return JSON.stringify({{ keys, records, removed, errors }});
}})()"#
    )
}

/// Which expected globals and scripts are present after a reload.
pub fn check(globals: &[String], scripts: &[String]) -> String {
    let g = serde_json::to_string(globals).unwrap();
    let s = serde_json::to_string(scripts).unwrap();
    format!(
        r#"(() => {{
  const srcs = [...document.scripts].map(x => decodeURIComponent((x.src || '').split('?')[0]));
  const g = {g}, s = {s};
  return JSON.stringify({{
    globals: Object.fromEntries(g.map(n => [n, typeof window[n] !== 'undefined'])),
    scripts: Object.fromEntries(s.map(n => [n, srcs.some(x => x.endsWith(n) || x === n)])),
    config: Spicetify.Config
  }});
}})()"#
    )
}

/// The self-test's round trip on a throwaway key.
pub fn selftest_roundtrip(key: &str) -> String {
    let k = serde_json::to_string(key).unwrap();
    format!(
        r#"(async () => {{ {CODEC}
  const k = {k};
  const sample = {{ d: new Date(5), a: new Uint8Array([1, 2, 255]), m: new Map([[1, 'x']]), s: new Set(['y']), n: NaN, $t: 'tricky', u: undefined }};
  const back = dec(JSON.parse(JSON.stringify(await enc(sample))));
  const codec = back.d.getTime() === 5 && back.a instanceof Uint8Array && back.a[2] === 255 && back.m.get(1) === 'x' && back.s.has('y') && Number.isNaN(back.n) && back.$t === 'tricky' && 'u' in back;
  localStorage.setItem(k, 'pantry');
  const wrote = localStorage.getItem(k) === 'pantry';
  localStorage.removeItem(k);
  return JSON.stringify({{ codec, wrote, removed: localStorage.getItem(k) === null }});
}})()"#
    )
}

/// Reloads the page (after a restore, so extensions start with the restored settings).
pub const RELOAD: &str = "setTimeout(() => location.reload(), 50), JSON.stringify(true)";
