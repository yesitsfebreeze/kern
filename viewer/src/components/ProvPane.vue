<script setup>
import KernIcon from '../KernIcon.vue'

const props = defineProps({
  entry: Object,
  thoughts: Object,
  reasons: Object,
  editing: Object,
  draft: String,
  stale: Boolean,
  busy: Boolean,
})
const emit = defineEmits(['startEdit', 'save', 'cancel', 'editKey', 'cut', 'setDraft', 'resend'])

function isEd(kind, id) { return props.editing?.kind === kind && props.editing?.id === id }

function sortedRetrieved() {
  return [...(props.entry?.msg?.retrieved || [])].sort((a, b) => b.score - a.score)
}
function usedSet() { return new Set(props.entry?.msg?.used || []) }
function usedReasons() {
  return (props.entry?.msg?.usedReasons || []).map(id => props.reasons[id]).filter(Boolean)
}

function onEditKey(e) {
  if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); emit('save') }
  if (e.key === 'Escape') { e.preventDefault(); emit('cancel') }
}
</script>

<template>
  <div class="prov">
    <div class="prov-scroll">
      <div class="prov-sec">
        <div class="prov-sec-head">
          <span class="lab">Pulled from memory</span>
          <span class="sub">injected as context</span>
          <span class="n">{{ sortedRetrieved().length }}</span>
        </div>
        <div v-for="r in sortedRetrieved()" :key="r.id">
          <div v-if="thoughts[r.id]" :class="['cand', usedSet().has(r.id) ? 'used' : '', r.cold ? 'cold' : '']">
            <div class="cand-top">
              <span class="cand-id">{{ r.id.slice(0,8) }}</span>
              <span v-if="r.cold" class="cand-flag cold"><KernIcon n="snow" :size="10" /> cold</span>
              <span v-if="usedSet().has(r.id)" class="cand-flag used"><KernIcon n="check" :size="10" /> leaned on</span>
              <span class="grow"></span>
              <span class="cand-score">
                <span class="score-bar"><i :style="{width: Math.round(r.score*100)+'%'}"></i></span>
                <span class="val">{{ r.score.toFixed(2) }}</span>
              </span>
            </div>
            <template v-if="isEd('thought', r.id)">
              <div class="edit-wrap">
                <textarea class="edit-area" rows="3" autofocus :value="draft"
                  @input="emit('setDraft', $event.target.value)"
                  @keydown="onEditKey"></textarea>
                <div class="edit-actions">
                  <button class="edit-save" @click="emit('save')"><KernIcon n="check" :size="13" /> Save thought</button>
                  <button class="edit-cancel" @click="emit('cancel')">Cancel</button>
                  <span class="edit-hint"><b>↵</b> save · <b>esc</b> cancel</span>
                </div>
              </div>
            </template>
            <template v-else>
              <div class="cand-text">{{ thoughts[r.id].text }}</div>
              <div class="cand-edit-row">
                <button class="mini-act" @click="emit('startEdit', 'thought', r.id, thoughts[r.id].text)">
                  <span class="ic"><KernIcon n="edit" :size="12" /></span> Edit thought
                </button>
              </div>
            </template>
          </div>
        </div>
      </div>

      <div class="prov-sec">
        <div class="prov-sec-head">
          <span class="lab">Why it connects</span>
          <span class="sub">reasons it leaned on</span>
          <span class="n">{{ usedReasons().length }}</span>
        </div>
        <div v-if="!usedReasons().length" class="cand">
          <div class="cand-text" style="color:var(--ink-3)">No connections used — this answer rested on a single thought.</div>
        </div>
        <div v-for="rs in usedReasons()" :key="rs.id" class="reason">
          <div class="reason-edge">
            <span class="edge-node">{{ rs.from }}</span>
            <span class="edge-arrow"><KernIcon n="arrowR" :size="13" /></span>
            <span class="edge-node">{{ rs.to }}</span>
          </div>
          <template v-if="isEd('reason', rs.id)">
            <div class="edit-wrap">
              <textarea class="edit-area" rows="3" autofocus :value="draft"
                @input="emit('setDraft', $event.target.value)"
                @keydown="onEditKey"></textarea>
              <div class="edit-actions">
                <button class="edit-save" @click="emit('save')"><KernIcon n="check" :size="13" /> Save reason</button>
                <button class="edit-cancel" @click="emit('cancel')">Cancel</button>
                <span class="edit-hint"><b>↵</b> save · <b>esc</b> cancel</span>
              </div>
            </div>
          </template>
          <template v-else>
            <div class="reason-why">{{ rs.why }}</div>
            <div class="cand-edit-row">
              <button class="mini-act" @click="emit('startEdit', 'reason', rs.id, rs.why)">
                <span class="ic"><KernIcon n="edit" :size="12" /></span> Edit reason
              </button>
              <button class="mini-act" @click="emit('cut', rs.id)">
                <span class="ic"><KernIcon n="cut" :size="12" /></span> Cut connection
              </button>
            </div>
          </template>
        </div>
      </div>
    </div>

    <div :class="['prov-foot', stale ? '' : 'quiet']">
      <div class="pf-row">
        <template v-if="stale">
          <span class="pf-tx"><b>You reshaped the memory.</b> Send this back through to rethink it.</span>
          <button class="resend" @click="emit('resend')">
            <template v-if="busy">rethinking…</template>
            <template v-else><KernIcon n="refresh" :size="14" /> Resend</template>
          </button>
        </template>
        <template v-else>
          <span class="pf-tx">Edit any thought or reason, then resend to steer the answer.</span>
        </template>
      </div>
    </div>
  </div>
</template>
