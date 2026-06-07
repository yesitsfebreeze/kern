<script setup>
import { ref, computed, onMounted, onBeforeUnmount, nextTick, watch } from 'vue'
import KernIcon from './KernIcon.vue'
import InboxPane from './components/InboxPane.vue'
import ConvPane from './components/ConvPane.vue'
import ProvPane from './components/ProvPane.vue'
import RelationsPane from './components/RelationsPane.vue'
import GraphPane from './components/GraphPane.vue'
import FlowPane from './components/FlowPane.vue'
import SettingsPane from './components/SettingsPane.vue'

// ── Constants ──
const ACCENTS = ['#CF5320', '#C2410C', '#B8455C', '#4F7A8C', '#5E7D5A', '#6D5B8A']
const KIND_ICON = { inbox: 'inbox', conv: 'pen', relations: 'relations', graph: 'graph', flow: 'flow', prov: 'layers', settings: 'sparkle', empty: 'grid' }
const KIND_LABEL = { inbox: 'inbox', conv: 'thread', relations: 'relations', graph: 'graph', flow: 'flow', prov: 'before · after', settings: 'settings', empty: 'empty' }
const COLW = (n) => n <= 1 ? [1] : n === 2 ? [1, 1] : [3, 4, 3]

let _k = 0
const uid = (p) => p + (++_k) + Date.now().toString(36).slice(-4)
const mkTab = (kind, convId) => ({ kind, convId })
function now() { const d = new Date(); return d.getHours()+':'+String(d.getMinutes()).padStart(2,'0') }

// ── State ──
const thoughts = ref({})
const reasons = ref({})
const convs = ref([])
const query = ref('')
const columns = ref([
  { id: 'c1', w: 3, rows: [
    { key: 't11', flex: 1, active: 0, tabs: [mkTab('inbox')] },
    { key: 't12', flex: 1, active: 0, tabs: [mkTab('relations')] },
  ]},
  { id: 'c2', w: 4, rows: [
    { key: 't21', flex: 1, active: 0, tabs: [mkTab('conv')] },
    { key: 't22', flex: 1, active: 0, tabs: [mkTab('flow')] },
  ]},
  { id: 'c3', w: 3, rows: [
    { key: 't31', flex: 1, active: 0, tabs: [mkTab('graph')] },
    { key: 't32', flex: 1, active: 0, tabs: [mkTab('prov')] },
  ]},
])
const focusKey = ref('t21')
const activeConv = ref(null)
const selId = ref(null)
const focusReplyBy = ref({})
const composeBy = ref({})
const editing = ref(null)
const draft = ref('')
const stale = ref({})
const resent = ref({})
const busyConv = ref(null)
const adding = ref(null)
const theme = ref('dark')
const density = ref('comfortable')
const accent = ref(ACCENTS[0])
const wmRef = ref(null)

// ── Daemon ──
let pulse = null
let historyMap = {}

async function loadGraph() {
  try {
    const g = await fetch('/graph').then(r => r.json())
    const ts = {}
    for (const n of g.nodes || []) {
      ts[n.id] = { id: n.id, text: n.text || n.label || n.id }
    }
    const rs = {}
    for (const l of g.links || []) {
      const src = typeof l.source === 'object' ? l.source.id : l.source
      const tgt = typeof l.target === 'object' ? l.target.id : l.target
      rs[l.id] = { id: l.id, from: src, to: tgt, why: l.text || l.label || l.why || '' }
    }
    thoughts.value = ts
    reasons.value = rs
  } catch (_) {
    // daemon offline — leave empty
  }
}

async function sendMessage(cid) {
  const txt = (composeBy.value[cid] || '').trim()
  if (!txt) return
  const uid_u = uid('u'), uid_r = uid('k')
  const userMsg = { id: uid_u, role: 'you', when: now(), text: [txt] }
  const kernReply = { id: uid_r, role: 'kern', when: now(), text: [''], retrieved: [], used: [], usedReasons: [] }
  convs.value = convs.value.map(c => c.id === cid ? { ...c, status: 'thinking', unread: false, messages: [...(c.messages||[]), userMsg, kernReply] } : c)
  composeBy.value = { ...composeBy.value, [cid]: '' }
  busyConv.value = cid
  const hist = historyMap[cid] || []

  try {
    const res = await fetch('/ask', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ question: txt, history: hist.slice(-6) }),
    })
    if (!res.ok || !res.body) throw new Error('unavailable')
    const reader = res.body.getReader()
    const dec = new TextDecoder()
    let buf = ''
    for (;;) {
      const { value, done } = await reader.read()
      if (done) break
      buf += dec.decode(value, { stream: true })
      let i
      while ((i = buf.indexOf('\n\n')) >= 0) {
        const frame = buf.slice(0, i); buf = buf.slice(i + 2)
        applyFrame(frame, cid, uid_r)
      }
    }
    hist.push({ role: 'user', content: txt })
    const reply = convs.value.find(c => c.id === cid)?.messages?.find(m => m.id === uid_r)
    if (reply) hist.push({ role: 'assistant', content: reply.text.join('\n') })
    historyMap[cid] = hist
    convs.value = convs.value.map(c => c.id === cid ? { ...c, status: 'replied', unread: false } : c)
  } catch (_) {
    // fallback: synthesize a reply from local thoughts
    const words = txt.toLowerCase().split(/\W+/).filter(w => w.length > 3)
    const ts = Object.values(thoughts.value)
    const ranked = ts.map(t => ({ id: t.id, score: words.filter(w => t.text.toLowerCase().includes(w)).length }))
      .sort((a, b) => b.score - a.score)
    const retrieved = ranked.slice(0, 5).map((r, i) => ({ id: r.id, score: Math.max(0.31, 0.93 - i * 0.13), cold: i >= 3 }))
    const used = retrieved.slice(0, 2).map(r => r.id)
    const bestTh = thoughts.value[used[0]]
    const replyText = bestTh
      ? [`Here's what I'm drawing on. The closest thing in your memory is: "${bestTh.text}"`, "Held against that, I'd stay true to it rather than chase the louder option — and if you disagree, edit the thought and send this back through."]
      : ["No relevant memory found yet. Ingest some thoughts first."]
    convs.value = convs.value.map(c => c.id === cid ? {
      ...c, status: 'replied', unread: false,
      messages: c.messages.map(m => m.id === uid_r ? { ...m, text: replyText, retrieved, used, usedReasons: [] } : m)
    } : c)
  } finally {
    busyConv.value = null
    focusReplyBy.value = { ...focusReplyBy.value, [cid]: uid_r }
    activeConv.value = cid
  }
}

function applyFrame(frame, cid, rid) {
  let ev = 'message', data = ''
  for (const line of frame.split('\n')) {
    if (line.startsWith('event:')) ev = line.slice(6).trim()
    else if (line.startsWith('data:')) data += (data ? '\n' : '') + line.slice(5).trim()
  }
  let d = {}
  try { d = data ? JSON.parse(data) : {} } catch (_) { return }
  if (ev === 'sources') {
    const retrieved = (d.entities || []).map((e, i) => ({ id: e.id, score: e.score || 0.5, cold: i >= 3 }))
    const used = retrieved.slice(0, 2).map(r => r.id)
    const usedReasons = (d.reasons || []).map(r => r.id)
    // Register any daemon reasons we don't have yet
    for (const r of (d.reasons || [])) {
      if (!reasons.value[r.id]) {
        reasons.value = { ...reasons.value, [r.id]: { id: r.id, from: r.from || '', to: r.to || '', why: r.text || r.why || '' } }
      }
    }
    convs.value = convs.value.map(c => c.id === cid ? {
      ...c, messages: c.messages.map(m => m.id === rid ? { ...m, retrieved, used, usedReasons } : m)
    } : c)
  } else if (ev === 'token') {
    convs.value = convs.value.map(c => c.id === cid ? {
      ...c, messages: c.messages.map(m => m.id === rid ? { ...m, text: m.text.length === 1 && m.text[0] === '' ? [d.t || ''] : [m.text.slice(0,-1).join('\n') + (d.t || '')] } : m)
    } : c)
  } else if (ev === 'error') {
    convs.value = convs.value.map(c => c.id === cid ? {
      ...c, messages: c.messages.map(m => m.id === rid ? { ...m, text: [...m.text, `⚠ ${d.message || 'error'}`] } : m)
    } : c)
  }
}

// ── Edit / stale ──
function editSave() {
  const txt = draft.value.trim()
  if (!txt || !editing.value) return
  const { kind, id } = editing.value
  if (kind === 'thought') {
    thoughts.value = { ...thoughts.value, [id]: { ...thoughts.value[id], text: txt } }
    // Mark stale: any kern reply that used this thought
    const n = {}
    for (const c of convs.value) {
      for (const m of c.messages || []) {
        if (m.role === 'kern' && (m.used || []).includes(id)) n[m.id] = true
      }
    }
    stale.value = { ...stale.value, ...n }
    // Send edit to daemon
    fetch('/edit', { method: 'POST', headers: {'content-type':'application/json'}, body: JSON.stringify({ id, text: txt, kind: 'entity' }) }).catch(() => {})
  } else {
    reasons.value = { ...reasons.value, [id]: { ...reasons.value[id], why: txt } }
    const n = {}
    for (const c of convs.value) {
      for (const m of c.messages || []) {
        if (m.role === 'kern' && (m.usedReasons || []).includes(id)) n[m.id] = true
      }
    }
    stale.value = { ...stale.value, ...n }
    fetch('/edit', { method: 'POST', headers: {'content-type':'application/json'}, body: JSON.stringify({ id, text: txt, kind: 'reason' }) }).catch(() => {})
  }
  editing.value = null
  draft.value = ''
}
function editCancel() { editing.value = null; draft.value = '' }
function startEdit(kind, id, text) { editing.value = { kind, id }; draft.value = text }
function cutReason(rid) {
  reasons.value = Object.fromEntries(Object.entries(reasons.value).filter(([k]) => k !== rid))
  const n = {}
  for (const c of convs.value) {
    for (const m of c.messages || []) {
      if (m.role === 'kern' && (m.usedReasons || []).includes(rid)) n[m.id] = true
    }
  }
  stale.value = { ...stale.value, ...n }
}

function resendReply(rid) {
  busyConv.value = rid
  setTimeout(() => {
    stale.value = Object.fromEntries(Object.entries(stale.value).filter(([k]) => k !== rid))
    resent.value = { ...resent.value, [rid]: true }
    busyConv.value = null
    setTimeout(() => {
      resent.value = Object.fromEntries(Object.entries(resent.value).filter(([k]) => k !== rid))
    }, 2600)
  }, 850)
}

// ── WM navigation ──
function allRows(cs) { return (cs || columns.value).flatMap(c => c.rows) }
function pos(key, cs) {
  const arr = cs || columns.value
  for (let ci = 0; ci < arr.length; ci++) {
    const ri = arr[ci].rows.findIndex(r => r.key === key)
    if (ri >= 0) return { ci, ri }
  }
  return null
}
function rowByKey(key, cs) {
  for (const c of (cs || columns.value)) {
    const r = c.rows.find(x => x.key === key)
    if (r) return r
  }
  return null
}
function activeTab(row) { return row.tabs[row.active] || row.tabs[0] }

function ensureVisible(key) {
  nextTick(() => {
    const wm = wmRef.value
    if (!wm) return
    const el = wm.querySelector(`[data-key="${key}"]`)
    if (!el) return
    const c = el.closest('.wm-col') || el
    const l = c.offsetLeft, r = l + c.offsetWidth
    if (l < wm.scrollLeft) wm.scrollLeft = l - 8
    else if (r > wm.scrollLeft + wm.clientWidth) wm.scrollLeft = r - wm.clientWidth + 8
  })
}

function setFocus(key) {
  focusKey.value = key
  const r = rowByKey(key)
  if (r) { const t = activeTab(r); if (t.kind === 'conv' && t.convId) activeConv.value = t.convId }
  ensureVisible(key)
}
function focusColRow(ci, ri) {
  const cs = columns.value
  if (!cs[ci]) return
  const row = cs[ci].rows[Math.min(ri, cs[ci].rows.length - 1)]
  if (row) setFocus(row.key)
}
function moveH(dir) {
  const p = pos(focusKey.value)
  if (!p) return
  const nci = Math.max(0, Math.min(columns.value.length - 1, p.ci + dir))
  focusColRow(nci, p.ri)
}
function moveV(dir) {
  const p = pos(focusKey.value)
  if (!p) return
  focusColRow(p.ci, Math.max(0, Math.min(columns.value[p.ci].rows.length - 1, p.ri + dir)))
}

let addrBuf = { col: null, t: 0 }
function pressDigit(d) {
  const now2 = Date.now()
  if (addrBuf.col && now2 - addrBuf.t < 850 && (d === 1 || d === 2)) {
    focusColRow(addrBuf.col - 1, d - 1)
    addrBuf = { col: null, t: 0 }
  } else if (d >= 1 && d <= 3) {
    focusColRow(d - 1, 0)
    addrBuf = { col: d, t: now2 }
  }
}

// ── Tile operations ──
function updRow(key, fn) {
  columns.value = columns.value.map(c => ({ ...c, rows: c.rows.map(r => r.key === key ? fn(r) : r) }))
}
function setActiveTab(key, ti) {
  updRow(key, r => ({ ...r, active: ti }))
  const r = rowByKey(key)
  const t = r?.tabs[ti]
  if (t?.kind === 'conv' && t.convId) activeConv.value = t.convId
}
function switchKind(key, kind) {
  updRow(key, r => {
    const tabs = r.tabs.slice()
    const cur = tabs[r.active]
    tabs[r.active] = { kind, convId: kind === 'conv' ? (cur.convId || activeConv.value || null) : undefined }
    return { ...r, tabs }
  })
  if (kind === 'conv') {
    const r = rowByKey(key)
    const cv = r?.tabs[r.active]?.convId
    if (cv) ensureFocusReply(cv)
  }
}
function addTab(key, kind) {
  updRow(key, r => ({ ...r, tabs: [...r.tabs, { kind, convId: kind === 'conv' ? (activeConv.value || null) : undefined }], active: r.tabs.length }))
  setFocus(key)
}
function closeTab(key, ti) {
  const r = rowByKey(key)
  if (!r) return
  if (r.tabs.length <= 1) { closePane(key); return }
  updRow(key, rr => {
    const tabs = rr.tabs.filter((_, i) => i !== ti)
    return { ...rr, tabs, active: Math.max(0, Math.min(rr.active - (ti <= rr.active ? 1 : 0), tabs.length - 1)) }
  })
}
function updActiveConv(key, cid) {
  updRow(key, r => { const tabs = r.tabs.slice(); tabs[r.active] = { ...tabs[r.active], convId: cid }; return { ...r, tabs } })
}

function emptyRowKey() {
  const e = allRows().find(r => activeTab(r).kind === 'empty')
  return e ? e.key : null
}
function focusedColIdx() { const p = pos(focusKey.value); return p ? p.ci : 1 }

function placePane(kind, convId) {
  const cs = columns.value
  if (kind !== 'conv') {
    const ex = allRows(cs).find(r => activeTab(r).kind === kind)
    if (ex) { setFocus(ex.key); return }
  }
  const ek = emptyRowKey()
  if (ek) { switchKind(ek, kind); if (convId) updActiveConv(ek, convId); setFocus(ek); return }
  const ci = focusedColIdx()
  if (cs[ci].rows.length < 2) {
    const key = uid('k')
    addRowTile(ci, kind, convId, key)
    nextTick(() => setFocus(key))
    return
  }
  addTab(focusKey.value, kind)
  if (convId) updActiveConv(focusKey.value, convId)
}
function addRowTile(ci, kind, convId, key) {
  columns.value = columns.value.map((c, i) => i === ci ? {
    ...c, rows: c.rows.length >= 2 ? c.rows : [
      ...c.rows.map(r => ({ ...r, flex: 1 })),
      { key: key || uid('k'), flex: 1, active: 0, tabs: [mkTab(kind, convId)] }
    ]
  } : c)
}

function ensureFocusReply(cid) {
  if (!focusReplyBy.value[cid]) {
    const c = convs.value.find(x => x.id === cid)
    const lk = c && [...(c.messages||[])].reverse().find(x => x.role === 'kern')
    if (lk) focusReplyBy.value = { ...focusReplyBy.value, [cid]: lk.id }
  }
}

function openThread(cid) {
  selId.value = cid
  activeConv.value = cid
  editing.value = null
  ensureFocusReply(cid)
  const fr = rowByKey(focusKey.value)
  if (fr && activeTab(fr).kind === 'conv') { updActiveConv(fr.key, cid); return }
  const firstConv = allRows().find(r => activeTab(r).kind === 'conv')
  if (firstConv) { updActiveConv(firstConv.key, cid); setFocus(firstConv.key); return }
  placePane('conv', cid)
}

function spawnConv() {
  const id = uid('th')
  convs.value = [{ id, subject: 'New conversation', status: 'draft', time: now(), unread: false, parties: 'you ⇄ kern', parentId: null, messages: [] }, ...convs.value]
  selId.value = id
  placePane('conv', id)
}

function branchConv() {
  const cur = convs.value.find(c => c.id === activeConv.value)
  if (!cur) return
  const id = uid('br')
  const parent = cur.parentId || cur.id
  convs.value = convs.value.reduce((acc, c) => {
    acc.push(c)
    if (c.id === cur.id) acc.push({ id, subject: cur.subject + ' — branch', status: 'draft', time: now(), unread: false, parties: cur.parties, parentId: parent, messages: [] })
    return acc
  }, [])
  selId.value = id
  placePane('conv', id)
}

function splitCol(ci) {
  if (columns.value[ci].rows.length >= 2) return
  const key = uid('k')
  addRowTile(ci, 'empty', undefined, key)
  nextTick(() => setFocus(key))
}

function closePane(key) {
  const p = pos(key)
  if (!p) return
  const cs = columns.value
  const col = cs[p.ci]
  if (col.rows.length > 1) {
    const sib = col.rows[p.ri === 0 ? 1 : 0].key
    columns.value = cs.map((c, i) => i === p.ci ? { ...c, rows: c.rows.filter(r => r.key !== key).map(r => ({ ...r, flex: 1 })) } : c)
    if (key === focusKey.value) nextTick(() => setFocus(sib))
  } else if (cs.length > 1) {
    const nci = p.ci > 0 ? p.ci - 1 : 1
    const sib = cs[nci].rows[0].key
    const next = cs.filter((_, i) => i !== p.ci)
    const w = COLW(next.length)
    columns.value = next.map((c, i) => ({ ...c, w: w[i] }))
    if (key === focusKey.value) nextTick(() => setFocus(sib))
  } else {
    updRow(key, r => ({ ...r, tabs: [mkTab('empty')], active: 0 }))
  }
}

function toggleKind(kind) {
  const ex = allRows().find(r => activeTab(r).kind === kind)
  if (ex) setFocus(ex.key); else placePane(kind)
}

function addAt(dir, key, kind) {
  const p = pos(key)
  if (!p) { adding.value = null; return }
  if (dir === 'tab') { addTab(key, kind); adding.value = null; return }
  const convId = kind === 'conv' ? (activeConv.value || null) : undefined
  const nk = uid('k')
  if (dir === 'left' || dir === 'right') {
    if (columns.value.length >= 3) { adding.value = null; return }
    const ncol = { id: uid('col'), w: 1, rows: [{ key: nk, flex: 1, active: 0, tabs: [mkTab(kind, convId)] }] }
    const out = columns.value.slice()
    out.splice(dir === 'left' ? p.ci : p.ci + 1, 0, ncol)
    const w = COLW(out.length)
    columns.value = out.map((c, i) => ({ ...c, w: w[i] }))
  } else {
    if (columns.value[p.ci].rows.length >= 2) { adding.value = null; return }
    columns.value = columns.value.map((c, i) => {
      if (i !== p.ci) return c
      const rows = c.rows.map(r => ({ ...r, flex: 1 }))
      rows.splice(dir === 'top' ? p.ri : p.ri + 1, 0, { key: nk, flex: 1, active: 0, tabs: [mkTab(kind, convId)] })
      return { ...c, rows }
    })
  }
  if (convId) { activeConv.value = convId; ensureFocusReply(convId) }
  adding.value = null
  nextTick(() => setFocus(nk))
}

// ── Resize ──
function startColResize(e, ci) {
  e.preventDefault(); e.stopPropagation()
  const wm = wmRef.value
  const els = wm.querySelectorAll('.wm-col')
  const aEl = els[ci], bEl = els[ci + 1]
  if (!bEl) return
  const a0 = aEl.getBoundingClientRect().width, b0 = bEl.getBoundingClientRect().width
  const sumW = (columns.value[ci].w || 1) + (columns.value[ci + 1].w || 1)
  const totalPx = a0 + b0
  const startX = e.clientX
  const g = e.currentTarget
  g.classList.add('drag')
  function move(ev) {
    const na = Math.max(60, Math.min(totalPx - 60, a0 + (ev.clientX - startX)))
    const lw = sumW * na / totalPx, rw = sumW - lw
    columns.value = columns.value.map((c, i) => i === ci ? { ...c, w: lw } : i === ci + 1 ? { ...c, w: rw } : c)
  }
  function up() {
    window.removeEventListener('pointermove', move)
    window.removeEventListener('pointerup', up)
    document.body.style.cursor = ''
    g.classList.remove('drag')
  }
  window.addEventListener('pointermove', move)
  window.addEventListener('pointerup', up)
  document.body.style.cursor = 'col-resize'
}

function startRowResize(e, ci, ri) {
  e.preventDefault(); e.stopPropagation()
  const wm = wmRef.value
  const colEl = wm.querySelectorAll('.wm-col')[ci]
  const tiles = colEl.querySelectorAll(':scope > .tile')
  const top = tiles[ri], bot = tiles[ri + 1]
  const startTop = top.getBoundingClientRect().height
  const avail = startTop + bot.getBoundingClientRect().height
  const startY = e.clientY
  const g = e.currentTarget
  g.classList.add('drag')
  function move(ev) {
    let nt = Math.max(110, Math.min(avail - 110, startTop + (ev.clientY - startY)))
    columns.value = columns.value.map((c, i) => {
      if (i !== ci) return c
      const rows = c.rows.slice()
      rows[ri] = { ...rows[ri], flex: nt }
      rows[ri + 1] = { ...rows[ri + 1], flex: avail - nt }
      return { ...c, rows }
    })
  }
  function up() {
    window.removeEventListener('pointermove', move)
    window.removeEventListener('pointerup', up)
    document.body.style.cursor = ''
    g.classList.remove('drag')
  }
  window.addEventListener('pointermove', move)
  window.addEventListener('pointerup', up)
  document.body.style.cursor = 'row-resize'
}

function resetCols() {
  const w = COLW(columns.value.length)
  columns.value = columns.value.map((c, i) => ({ ...c, w: w[i] }))
}

// ── Computed inbox lists ──
const topLevel = computed(() => convs.value.filter(c => c.parentId == null))
const branchesOf = (id) => convs.value.filter(c => c.parentId === id)
const unsettled = (x) => x.status !== 'replied' || x.unread
const isActive = (c) => unsettled(c) || branchesOf(c.id).some(unsettled)
const qq = computed(() => query.value.trim().toLowerCase())
const matchesC = (c) => !qq.value || (c.subject + ' ' + (c.messages||[]).map(m => (Array.isArray(m.text)?m.text:m.text?[m.text]:[]).join(' ')).join(' ')).toLowerCase().includes(qq.value)
const activeThreads = computed(() => topLevel.value.filter(c => isActive(c) && (matchesC(c) || branchesOf(c.id).some(matchesC))))
const resolvedThreads = computed(() => topLevel.value.filter(c => !isActive(c) && (matchesC(c) || branchesOf(c.id).some(matchesC))))

// ── Active reply provenance ──
const activeReply = computed(() => {
  const c = convs.value.find(x => x.id === activeConv.value)
  const rid = c && focusReplyBy.value[c.id]
  if (!rid) return null
  for (const conv of convs.value) {
    const m = conv.messages?.find(x => x.id === rid)
    if (m) return m
  }
  return null
})
const activeUsed = computed(() => new Set(activeReply.value?.used || []))

// ── Keyboard ──
function onKey(e) {
  const tag = (document.activeElement?.tagName) || ''
  const inField = tag === 'INPUT' || tag === 'TEXTAREA'
  if (e.altKey) {
    const k = e.key.toLowerCase()
    const map = {
      arrowleft: () => moveH(-1), h: () => moveH(-1),
      arrowright: () => moveH(1), l: () => moveH(1),
      arrowup: () => moveV(-1), k: () => moveV(-1),
      arrowdown: () => moveV(1), j: () => moveV(1),
      n: spawnConv,
      b: branchConv,
      p: () => toggleKind('prov'),
      s: () => splitCol(focusedColIdx()),
      t: () => { adding.value = { key: focusKey.value, dir: 'tab' } },
      ',': () => toggleKind('settings'),
      w: () => closePane(focusKey.value),
    }
    if (map[k]) { e.preventDefault(); map[k]() }
    return
  }
  if (e.key === 'Escape' && inField) { document.activeElement.blur(); return }
  if (!inField) {
    if (/^[1-3]$/.test(e.key)) { e.preventDefault(); pressDigit(+e.key); return }
  }
}

// ── Accent watcher ──
watch(accent, (v) => { document.documentElement.style.setProperty('--ember', v) }, { immediate: true })

// ── Lifecycle ──
onMounted(() => {
  loadGraph()
  pulse = setInterval(loadGraph, 5000)
  window.addEventListener('keydown', onKey)
})
onBeforeUnmount(() => {
  if (pulse) clearInterval(pulse)
  window.removeEventListener('keydown', onKey)
})

// ── Tab label helpers ──
function tabLabel(t) {
  if (t.kind === 'conv') {
    const c = convs.value.find(x => x.id === t.convId)
    return c?.subject || 'thread'
  }
  return KIND_LABEL[t.kind] || t.kind
}

// ── Body renderer per tile ──
function tileKind(row) { return (row.tabs[row.active] || row.tabs[0]).kind }

const PLUGIN_OPTS = [
  ['inbox', 'inbox'], ['conv', 'thread'], ['relations', 'relations'],
  ['graph', 'graph'], ['flow', 'flow'], ['prov', 'before · after'],
]

const flatTiles = computed(() =>
  columns.value.flatMap((col, ci) => col.rows.map((row, ri) => ({ ci, ri, row })))
)
</script>

<template>
  <div class="app" :data-theme="theme" :data-density="density">
    <!-- Topbar -->
    <header class="topbar">
      <div class="brand">
        <span class="mk"><KernIcon n="layers" :size="15" /></span>
        <span class="nm">kern</span>
        <span class="tag">workspace</span>
      </div>
      <div class="grow"></div>
      <label class="topsearch">
        <KernIcon n="search" :size="15" />
        <input v-model="query" placeholder="search threads + memory" />
      </label>
      <div class="grow"></div>
      <button class="icon-btn" title="Add window (alt+t)" @click="adding = { key: focusKey, dir: 'tab' }">
        <KernIcon n="plus" :size="18" />
      </button>
      <button class="icon-btn" title="Settings (alt+,)" @click="toggleKind('settings')">
        <KernIcon n="sparkle" :size="17" />
      </button>
      <button class="btn primary" @click="spawnConv">
        <KernIcon n="pen" :size="15" /> Compose
      </button>
    </header>

    <!-- Workspace -->
    <div class="wm" ref="wmRef">
      <template v-for="(col, ci) in columns" :key="col.id">
        <div class="wm-col" :style="{ flex: (col.w||1) + ' 1 0' }">
          <template v-for="(row, ri) in col.rows" :key="row.key">
            <section
              :data-key="row.key"
              :class="['tile', 'kind-'+tileKind(row), row.key===focusKey ? 'focused' : '']"
              :style="{ flex: (row.flex||1) + ' 1 0', minHeight: 0 }"
              @mousedown="setFocus(row.key)"
            >
              <!-- Tile header -->
              <header class="tile-bar">
                <span class="tile-addr">
                  {{ (ci+1)+'.'+( ri+1) }}
                </span>
                <div class="tile-tabs">
                  <button
                    v-for="(t, ti) in row.tabs" :key="ti"
                    :class="['ttab', ti===row.active ? 'on' : '']"
                    @mousedown.stop
                    @click.stop="setActiveTab(row.key, ti)"
                  >
                    <span class="tmic"><KernIcon :n="KIND_ICON[t.kind]||'grid'" :size="13" /></span>
                    <span class="tlab">{{ tabLabel(t) }}</span>
                    <span v-if="row.tabs.length>1" class="tx" @click.stop="closeTab(row.key, ti)">
                      <KernIcon n="x" :size="11" />
                    </span>
                  </button>
                  <button class="tadd" title="Add tab (alt+t)" @mousedown.stop @click.stop="adding = {key: row.key, dir: 'tab'}">
                    <KernIcon n="plus" :size="13" />
                  </button>
                </div>
                <span class="tb-actions">
                  <!-- branch button if conv -->
                  <template v-if="tileKind(row)==='conv' && row.tabs[row.active]?.convId">
                    <span class="tb-status">
                      <span :class="['status', convs.find(c=>c.id===row.tabs[row.active].convId)?.status||'draft']">
                        <span class="dot"></span>{{ convs.find(c=>c.id===row.tabs[row.active].convId)?.status||'draft' }}
                      </span>
                    </span>
                    <button class="tb-btn" @mousedown.stop @click.stop="activeConv=row.tabs[row.active].convId; branchConv()" title="Branch (alt+b)">
                      <KernIcon n="branch" :size="14" />
                    </button>
                  </template>
                  <button v-if="col.rows.length<2" class="tile-x" title="Split (alt+s)" @mousedown.stop @click.stop="splitCol(ci)">
                    <KernIcon n="panelR" :size="15" />
                  </button>
                  <button class="tile-x" title="Close (alt+w)" @mousedown.stop @click.stop="closePane(row.key)">
                    <KernIcon n="x" :size="14" />
                  </button>
                </span>
              </header>

              <!-- Tile body -->
              <div class="tile-body">
                <!-- inbox -->
                <InboxPane
                  v-if="tileKind(row)==='inbox'"
                  :activeThreads="activeThreads"
                  :resolvedThreads="resolvedThreads"
                  :branchesOf="branchesOf"
                  :selId="selId"
                  :count="activeThreads.length"
                  @openThread="openThread"
                  @spawnConv="spawnConv"
                />

                <!-- conversation -->
                <template v-else-if="tileKind(row)==='conv'">
                  <ConvPane
                    v-if="convs.find(c=>c.id===row.tabs[row.active]?.convId)"
                    :conv="convs.find(c=>c.id===row.tabs[row.active].convId)"
                    :focusReply="focusReplyBy[row.tabs[row.active].convId]"
                    :stale="stale"
                    :resent="resent"
                    :busy="busyConv"
                    :compose="composeBy[row.tabs[row.active].convId] || ''"
                    @setFocusReply="rid => { focusReplyBy = {...focusReplyBy, [row.tabs[row.active].convId]: rid}; activeConv = row.tabs[row.active].convId }"
                    @setCompose="v => composeBy = {...composeBy, [row.tabs[row.active].convId]: v}"
                    @send="sendMessage(row.tabs[row.active].convId)"
                    @resend="resendReply"
                  />
                  <!-- launcher if no conv selected -->
                  <div v-else class="launcher">
                    <div class="lc-t">No conversation selected</div>
                    <div class="lc-s">pick one from the inbox or compose a new one</div>
                    <div class="lc-grid">
                      <button v-for="[k,l] in PLUGIN_OPTS" :key="k" class="lc-btn" @click="switchKind(row.key, k)">
                        <span class="ic"><KernIcon :n="KIND_ICON[k]" :size="14" /></span>{{ l }}
                      </button>
                    </div>
                  </div>
                </template>

                <!-- prov / before-after -->
                <template v-else-if="tileKind(row)==='prov'">
                  <ProvPane
                    v-if="activeReply"
                    :entry="{ msg: activeReply }"
                    :thoughts="thoughts"
                    :reasons="reasons"
                    :editing="editing"
                    :draft="draft"
                    :stale="!!stale[activeReply.id]"
                    :busy="busyConv === activeReply.id"
                    @startEdit="startEdit"
                    @save="editSave"
                    @cancel="editCancel"
                    @setDraft="v => draft = v"
                    @cut="cutReason"
                    @resend="() => resendReply(activeReply.id)"
                  />
                  <div v-else class="thread-empty" style="padding:26px">
                    <span class="te-mk"><KernIcon n="layers" :size="24" /></span>
                    <div class="te-t" style="font-size:16px">Before &amp; after</div>
                    <div class="te-s">Open a reply to see what kern pulled and leaned on.</div>
                  </div>
                </template>

                <!-- relations -->
                <RelationsPane
                  v-else-if="tileKind(row)==='relations'"
                  :thoughts="thoughts"
                  :reasons="reasons"
                  :editing="editing"
                  :draft="draft"
                  :activeUsed="activeUsed"
                  @startEdit="startEdit"
                  @save="editSave"
                  @cancel="editCancel"
                  @setDraft="v => draft = v"
                  @cut="cutReason"
                />

                <!-- graph -->
                <GraphPane
                  v-else-if="tileKind(row)==='graph'"
                  :thoughts="thoughts"
                  :reasons="reasons"
                  :activeUsed="activeUsed"
                />

                <!-- flow -->
                <FlowPane
                  v-else-if="tileKind(row)==='flow'"
                  :thoughts="thoughts"
                  :reasons="reasons"
                  :editing="editing"
                  :draft="draft"
                  @startEdit="startEdit"
                  @save="editSave"
                  @cancel="editCancel"
                  @setDraft="v => draft = v"
                />

                <!-- settings -->
                <SettingsPane
                  v-else-if="tileKind(row)==='settings'"
                  :theme="theme"
                  :density="density"
                  :accent="accent"
                  @setTheme="v => theme = v"
                  @setDensity="v => density = v"
                  @setAccent="v => accent = v"
                />

                <!-- empty / launcher -->
                <div v-else class="launcher">
                  <div class="lc-t">Empty tile</div>
                  <div class="lc-s">display any window here</div>
                  <div class="lc-grid">
                    <button v-for="[k,l] in PLUGIN_OPTS" :key="k" class="lc-btn" @click="switchKind(row.key, k)">
                      <span class="ic"><KernIcon :n="KIND_ICON[k]" :size="14" /></span>{{ l }}
                    </button>
                  </div>
                </div>
              </div>

              <!-- Edge add buttons -->
              <div class="tile-edges">
                <div v-if="col.rows.length<2" class="edge-zone ez-top">
                  <button class="edge-plus" title="Add above" @mousedown.stop @click.stop="adding={key:row.key,dir:'top'}">
                    <KernIcon n="plus" :size="15" />
                  </button>
                </div>
                <div v-if="col.rows.length<2" class="edge-zone ez-bottom">
                  <button class="edge-plus" title="Add below" @mousedown.stop @click.stop="adding={key:row.key,dir:'bottom'}">
                    <KernIcon n="plus" :size="15" />
                  </button>
                </div>
                <div v-if="columns.length<3" class="edge-zone ez-left">
                  <button class="edge-plus" title="Add column left" @mousedown.stop @click.stop="adding={key:row.key,dir:'left'}">
                    <KernIcon n="plus" :size="15" />
                  </button>
                </div>
                <div v-if="columns.length<3" class="edge-zone ez-right">
                  <button class="edge-plus" title="Add column right" @mousedown.stop @click.stop="adding={key:row.key,dir:'right'}">
                    <KernIcon n="plus" :size="15" />
                  </button>
                </div>
              </div>

              <!-- Add chooser overlay -->
              <div v-if="adding && adding.key===row.key" class="add-cover" @mousedown.stop="adding=null">
                <div class="add-chooser" @mousedown.stop>
                  <span class="ey ac-lab">add window {{ {tab:'as a tab here',top:'above',bottom:'below',left:'to the left',right:'to the right'}[adding.dir] }} · pick a plugin</span>
                  <div class="lc-grid">
                    <button v-for="[k,l] in PLUGIN_OPTS" :key="k" class="lc-btn" @click="addAt(adding.dir, row.key, k)">
                      <span class="ic"><KernIcon :n="KIND_ICON[k]" :size="14" /></span>{{ l }}
                    </button>
                  </div>
                  <span class="ac-hint">more plugins soon</span>
                </div>
              </div>
            </section>

            <!-- Row gutter -->
            <div v-if="ri < col.rows.length-1" class="row-gutter" title="Drag to resize"
              @pointerdown="startRowResize($event, ci, ri)"><span class="g"></span></div>
          </template>
        </div>

        <!-- Column gutter -->
        <div v-if="ci < columns.length-1" class="col-gutter" title="Drag to resize · double-click to reset"
          @pointerdown="startColResize($event, ci)"
          @dblclick="resetCols"><span class="g"></span></div>
      </template>
    </div>

    <!-- Statusbar -->
    <footer class="statusbar">
      <span class="sb-ws"><span class="d"></span>kern</span>
      <div class="sb-tiles">
        <button
          v-for="{ ci, ri, row } in flatTiles"
          :key="row.key"
          :class="['sb-chip', row.key===focusKey ? 'on' : '']"
          @click="setFocus(row.key)"
        >
          <span class="n">{{ (ci+1)+'.'+(ri+1) }}</span>
          <span class="lbl">{{ tabLabel(activeTab(row)) }}</span>
        </button>
      </div>
      <span class="grow"></span>
      <span class="sb-hint"><b>1–3</b> then <b>1/2</b> address · <b>alt+←↑↓→</b> move · <b>alt+t</b> tab · <b>alt+s</b> split · <b>alt+n</b> new · <b>alt+w</b> close</span>
    </footer>
  </div>
</template>
