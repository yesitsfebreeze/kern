<script setup>
import KernIcon from '../KernIcon.vue'

const props = defineProps({
  thoughts: Object,
  reasons: Object,
  editing: Object,
  draft: String,
})
const emit = defineEmits(['startEdit', 'save', 'cancel', 'setDraft'])

function isEd(id) { return props.editing?.kind === 'reason' && props.editing?.id === id }
function onEditKey(e) {
  if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); emit('save') }
  if (e.key === 'Escape') { e.preventDefault(); emit('cancel') }
}
function nodeText(id) {
  return props.thoughts[id]?.text || id
}
</script>

<template>
  <div class="flow">
    <div v-for="(r, i) in Object.values(reasons)" :key="r.id" class="flow-step">
      <div class="flow-node">{{ nodeText(r.from) }}</div>
      <div class="flow-why">
        <span class="fic"><KernIcon n="flow" :size="14" /></span>
        <template v-if="isEd(r.id)">
          <div style="flex:1">
            <div class="edit-wrap">
              <textarea class="edit-area" rows="3" autofocus :value="draft"
                @input="emit('setDraft', $event.target.value)"
                @keydown="onEditKey"></textarea>
              <div class="edit-actions">
                <button class="edit-save" @click="emit('save')"><KernIcon n="check" :size="13" /> Save reason</button>
                <button class="edit-cancel" @click="emit('cancel')">Cancel</button>
              </div>
            </div>
          </div>
        </template>
        <template v-else>
          <span class="fwt" @click="emit('startEdit', 'reason', r.id, r.why)" title="Click to edit">{{ r.why }}</span>
        </template>
      </div>
      <div class="flow-node" style="border-color:var(--ember-line)">{{ nodeText(r.to) }}</div>
      <div v-if="i < Object.values(reasons).length - 1" style="height:14px"></div>
    </div>
    <div v-if="!Object.values(reasons).length" class="thread-empty" style="padding:26px">
      <span class="te-mk"><KernIcon n="flow" :size="24" /></span>
      <div class="te-t" style="font-size:16px">flow</div>
      <div class="te-s">No connections in memory yet.</div>
    </div>
  </div>
</template>
