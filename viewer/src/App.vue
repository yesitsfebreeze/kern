<script setup>
import { ref, onMounted, onBeforeUnmount } from 'vue'
import * as d3 from 'd3'

const stats = ref('loading…')
const err = ref('')
const crumbs = ref([])
const detail = ref('')
const svgEl = ref(null)

let timer = null
let raw = { nodes: [], links: [], kerns: [] }
let kernsById = {}
let entsByKern = new Map()
let treeData = null
let stack = []          // focus path (data nodes); last = current
let lastTopo = ''
let wheelLock = 0

const KIND_COLOR = { Fact: '#e5c07b', Document: '#61afef', Question: '#c678dd', Claim: '#98c379' }

function rootId() {
  const r = raw.kerns.find(k => !k.parent) || raw.kerns[0]
  return r ? r.id : null
}

// Nested data: kern → child kerns → … → thoughts (leaves, value 1).
function buildTree() {
  const make = (kid, seen) => {
    const k = kernsById[kid]
    if (!k || seen.has(kid)) return null
    seen.add(kid)
    const node = { id: kid, label: k.named ? k.label : '(unnamed)', type: 'kern', children: [] }
    for (const c of k.children || []) { const cn = make(c, seen); if (cn) node.children.push(cn) }
    for (const e of entsByKern.get(kid) || [])
      node.children.push({ id: e.id, label: e.label, type: 'entity', kind: e.kind, heat: e.heat, conf: e.conf, value: 1 })
    return node
  }
  return make(rootId(), new Set()) || { id: 'root', label: 'root', type: 'kern', children: [] }
}

function findById(node, id) {
  if (node.id === id) return node
  for (const c of node.children || []) { const r = findById(c, id); if (r) return r }
  return null
}

const W = () => svgEl.value.clientWidth
const H = () => svgEl.value.clientHeight

let svg, g
function render() {
  const cur = stack[stack.length - 1]
  crumbs.value = stack.map((n, i) => ({ id: n.id, label: i === 0 ? 'root' : n.label }))

  const r = d3.hierarchy(cur).sum(d => d.value || 0).sort((a, b) => (b.value || 0) - (a.value || 0))
  d3.treemap().size([W(), H()]).paddingOuter(2).paddingTop(d => d.depth === 0 ? 0 : 18).paddingInner(1).round(true)(r)

  g.selectAll('*').remove()

  // leaf cells = thoughts, coloured by kind — "all the data" at once.
  g.selectAll('rect.leaf').data(r.leaves()).join('rect').attr('class', 'leaf')
    .attr('x', d => d.x0).attr('y', d => d.y0)
    .attr('width', d => Math.max(0, d.x1 - d.x0)).attr('height', d => Math.max(0, d.y1 - d.y0))
    .attr('fill', d => d.data.type === 'entity' ? (KIND_COLOR[d.data.kind] || '#98c379') : '#222a33')
    .attr('fill-opacity', 0.85)
    .on('mousemove', (ev, d) => { detail.value = `${d.data.kind || ''} ${d.data.label}` })
    .on('mouseleave', () => detail.value = '')
    .append('title').text(d => d.data.label)

  // depth-1 blocks = direct sub-topics; bordered + labeled = the "word matrix".
  const blocks = (r.children || []).filter(d => d.data.type === 'kern')
  const blk = g.selectAll('g.blk').data(blocks).join('g').attr('class', 'blk')
  blk.append('rect')
    .attr('x', d => d.x0).attr('y', d => d.y0)
    .attr('width', d => Math.max(0, d.x1 - d.x0)).attr('height', d => Math.max(0, d.y1 - d.y0))
    .attr('fill', 'none').attr('stroke', '#7fd1ae').attr('stroke-opacity', 0.5).attr('stroke-width', 1)
    .style('cursor', 'pointer')
    .on('click', (ev, d) => zoomIn(d.data))
  blk.append('text')
    .attr('x', d => d.x0 + 4).attr('y', d => d.y0 + 13)
    .attr('fill', '#cfe9df').attr('font-size', '12px').attr('font-weight', 600)
    .style('pointer-events', 'none')
    .text(d => (d.x1 - d.x0) > 40 ? `${d.data.label}  (${d.value})` : '')

  stats.value = `${raw.nodes.length} thoughts · ${raw.kerns.length} spheres · here: ${r.value} thoughts`
}

function zoomIn(dataNode) {
  if (dataNode.type !== 'kern' || !(dataNode.children || []).some(c => c.type === 'kern' || c.type === 'entity')) return
  if (dataNode.id === stack[stack.length - 1].id) return
  stack.push(dataNode)
  render()
}
function zoomOut() { if (stack.length > 1) { stack.pop(); render() } }
function goTo(id) {
  const i = stack.findIndex(n => n.id === id)
  if (i >= 0) { stack.length = i + 1; render() }
}

// Scroll = narrow into / back out of the tree.
function onWheel(ev) {
  ev.preventDefault()
  const now = Date.now()
  if (now - wheelLock < 350) return
  wheelLock = now
  if (ev.deltaY > 0) {
    // into the sub-topic under the cursor
    const cur = stack[stack.length - 1]
    const r = d3.hierarchy(cur).sum(d => d.value || 0).sort((a, b) => (b.value || 0) - (a.value || 0))
    d3.treemap().size([W(), H()]).paddingOuter(2).paddingTop(d => d.depth === 0 ? 0 : 18).paddingInner(1).round(true)(r)
    const [mx, my] = d3.pointer(ev, svgEl.value)
    const hit = (r.children || []).find(d => d.data.type === 'kern' && mx >= d.x0 && mx <= d.x1 && my >= d.y0 && my <= d.y1)
    if (hit) zoomIn(hit.data)
  } else {
    zoomOut()
  }
}

async function load() {
  try {
    raw = await (await fetch('/graph')).json()
    kernsById = {}; entsByKern = new Map()
    for (const k of raw.kerns) kernsById[k.id] = k
    for (const e of raw.nodes) { if (!entsByKern.has(e.kern)) entsByKern.set(e.kern, []); entsByKern.get(e.kern).push(e) }
    const topo = raw.nodes.length + ':' + raw.kerns.length
    if (topo !== lastTopo) {
      lastTopo = topo
      treeData = buildTree()
      // rebuild the focus stack against the new tree (keep where you were)
      const ids = stack.map(n => n.id)
      stack = [treeData]
      for (let i = 1; i < ids.length; i++) { const n = findById(treeData, ids[i]); if (n) stack.push(n); else break }
      render()
    }
    err.value = ''
  } catch (e) { err.value = String(e) }
}

onMounted(() => {
  svg = d3.select(svgEl.value)
  g = svg.append('g')
  svgEl.value.addEventListener('wheel', onWheel, { passive: false })
  load()
  timer = setInterval(load, 5000)
  window.addEventListener('resize', () => render())
})
onBeforeUnmount(() => { if (timer) clearInterval(timer); svgEl.value?.removeEventListener('wheel', onWheel) })
</script>

<template>
  <div class="hud">
    <b>kern</b>
    <span class="crumbs">
      <template v-for="(c, i) in crumbs" :key="c.id">
        <a @click="goTo(c.id)" :class="{ here: i === crumbs.length - 1 }">{{ c.label }}</a>
        <span v-if="i < crumbs.length - 1" class="sep"> › </span>
      </template>
    </span>
    <span class="stat">· {{ stats }}</span>
    <span v-if="err" class="err"> — {{ err }}</span>
  </div>
  <div class="legend">
    <span style="color:#98c379">■ claim</span>
    <span style="color:#e5c07b">■ fact</span>
    <span style="color:#61afef">■ document</span>
    <span style="color:#c678dd">■ question</span>
    <span class="dim">· scroll ↓ into a topic · scroll ↑ out</span>
  </div>
  <div class="path">{{ detail }}</div>
  <svg ref="svgEl" class="tm"></svg>
</template>

<style>
html, body, #app { height: 100%; }
.tm { position: fixed; inset: 0; width: 100vw; height: 100vh; background: #06080b; }
.hud {
  position: fixed; top: 10px; left: 12px; z-index: 10; background: #11151aee; color: #cdd3da;
  padding: 8px 12px; border-radius: 8px; border: 1px solid #222a33;
  font: 13px system-ui, sans-serif; display: flex; gap: 8px; align-items: center; flex-wrap: wrap; max-width: 90vw;
}
.hud b { color: #7fd1ae; }
.crumbs a { color: #8fb6d8; cursor: pointer; }
.crumbs a.here { color: #e5c07b; font-weight: 600; }
.crumbs .sep { color: #4a5563; }
.stat { color: #9aa6b2; }
.err { color: #e06c75; }
.legend {
  position: fixed; top: 50px; left: 12px; z-index: 10; background: #11151acc; color: #9aa6b2;
  padding: 4px 10px; border-radius: 6px; font: 12px system-ui, sans-serif; display: flex; gap: 12px;
}
.legend .dim { color: #5a6573; }
.path {
  position: fixed; bottom: 10px; left: 12px; right: 12px; z-index: 10; color: #cdd3da;
  font: 13px system-ui, sans-serif; background: #11151acc; padding: 6px 12px; border-radius: 6px;
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis; min-height: 16px;
}
</style>
