<script setup>
import { ref } from 'vue'

const props = defineProps({
  thoughts: Object,
  reasons: Object,
  activeUsed: Object,
})

const tf = ref({ x: 0, y: 0, k: 1 })
const drag = ref(null)
const graphEl = ref(null)

const cx = 190, cy = 150, R = 110

function ptOf() {
  const ids = Object.keys(props.thoughts)
  const N = ids.length
  const pts = {}
  ids.forEach((id, i) => {
    const a = (i / Math.max(N, 1)) * Math.PI * 2 - Math.PI / 2
    const jit = ((i * 37) % 7) - 3
    pts[id] = { x: cx + Math.cos(a) * (R + jit * 3), y: cy + Math.sin(a) * (R + jit * 2) }
  })
  return pts
}

function down(e) {
  drag.value = { x: e.clientX, y: e.clientY, ox: tf.value.x, oy: tf.value.y }
  e.currentTarget.classList.add('drag')
}
function move(e) {
  if (!drag.value) return
  tf.value = { ...tf.value, x: drag.value.ox + (e.clientX - drag.value.x), y: drag.value.oy + (e.clientY - drag.value.y) }
}
function up(e) {
  drag.value = null
  e.currentTarget.classList.remove('drag')
}
function wheel(e) {
  e.preventDefault()
  tf.value = { ...tf.value, k: Math.max(0.5, Math.min(2.4, tf.value.k * (e.deltaY < 0 ? 1.1 : 0.9))) }
}
</script>

<template>
  <div class="graph" ref="graphEl"
    @pointerdown="down" @pointermove="move" @pointerup="up" @pointerleave="up" @wheel.prevent="wheel">
    <svg>
      <g :transform="`translate(${tf.x},${tf.y}) scale(${tf.k})`">
        <template v-for="r in Object.values(reasons)" :key="r.id">
          <line v-if="ptOf()[r.from] && ptOf()[r.to]"
            class="ge"
            :x1="ptOf()[r.from]?.x" :y1="ptOf()[r.from]?.y"
            :x2="ptOf()[r.to]?.x" :y2="ptOf()[r.to]?.y"
          />
        </template>
        <g v-for="(id) in Object.keys(thoughts)" :key="id"
          :class="['gn', activeUsed?.has(id) ? 'used' : '']"
          :transform="`translate(${ptOf()[id]?.x},${ptOf()[id]?.y})`"
        >
          <circle :r="activeUsed?.has(id) ? 9 : 6.5" />
          <text text-anchor="middle" y="-12">{{ id.slice(0, 6) }}</text>
        </g>
      </g>
    </svg>
    <span class="graph-hint">drag to pan · scroll to zoom</span>
  </div>
</template>
