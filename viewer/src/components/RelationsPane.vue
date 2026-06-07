<script setup>
import KernIcon from '../KernIcon.vue'

const props = defineProps({
  thoughts: Object,
  reasons: Object,
  editing: Object,
  draft: String,
  activeUsed: Object,
})
const emit = defineEmits(['startEdit', 'save', 'cancel', 'cut', 'setDraft'])

function isEd(kind, id) { return props.editing?.kind === kind && props.editing?.id === id }
function onEditKey(e) {
  if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); emit('save') }
  if (e.key === 'Escape') { e.preventDefault(); emit('cancel') }
}
</script>

<template>
  <div class="rel">
    <div class="rel-sec">
      <span class="ey">Reasons · connections</span>
      <div v-for="r in Object.values(reasons)" :key="r.id" class="reason">
        <div class="reason-edge">
          <span class="edge-node">{{ r.from }}</span>
          <span class="edge-arrow"><KernIcon n="arrowR" :size="13" /></span>
          <span class="edge-node">{{ r.to }}</span>
        </div>
        <template v-if="isEd('reason', r.id)">
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
          <div class="reason-why">{{ r.why }}</div>
          <div class="cand-edit-row">
            <button class="mini-act" @click="emit('startEdit', 'reason', r.id, r.why)">
              <span class="ic"><KernIcon n="edit" :size="12" /></span> Edit
            </button>
            <button class="mini-act" @click="emit('cut', r.id)">
              <span class="ic"><KernIcon n="cut" :size="12" /></span> Cut
            </button>
          </div>
        </template>
      </div>
    </div>
    <div class="rel-sec" style="margin-top:16px">
      <span class="ey">Thoughts · atoms</span>
      <div v-for="t in Object.values(thoughts)" :key="t.id"
        class="rel-thought"
        :style="activeUsed?.has(t.id) ? {borderColor:'var(--ember-line)'} : null"
      >
        <span class="rid">{{ t.id.slice(0,6) }}</span>
        <template v-if="isEd('thought', t.id)">
          <div style="flex:1">
            <div class="edit-wrap">
              <textarea class="edit-area" rows="3" autofocus :value="draft"
                @input="emit('setDraft', $event.target.value)"
                @keydown="onEditKey"></textarea>
              <div class="edit-actions">
                <button class="edit-save" @click="emit('save')"><KernIcon n="check" :size="13" /> Save thought</button>
                <button class="edit-cancel" @click="emit('cancel')">Cancel</button>
              </div>
            </div>
          </div>
        </template>
        <template v-else>
          <span class="rtx">{{ t.text }}
            <button class="mini-act rel-edit" @click="emit('startEdit', 'thought', t.id, t.text)">
              <span class="ic"><KernIcon n="edit" :size="12" /></span>
            </button>
          </span>
        </template>
      </div>
    </div>
  </div>
</template>
