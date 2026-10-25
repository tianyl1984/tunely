<script setup>
import { onMounted, onBeforeUnmount, ref, useId } from "vue";
import Icon from "./Icon.vue";
defineProps({
  title: String,
  subtitle: String,
  wide: Boolean,
  busy: Boolean,
  closeOnBackdrop: { type: Boolean, default: true },
});
const emit = defineEmits(["close"]);
const dialog = ref(null);
const titleId = useId();
onMounted(() => dialog.value.showModal());
onBeforeUnmount(() => dialog.value?.close());
</script>
<template>
  <Teleport to="body">
    <dialog
      ref="dialog"
      class="modal"
      :class="{ wide }"
      :aria-labelledby="titleId"
      @cancel.prevent="!busy && emit('close')"
      @click="
        (event) =>
          closeOnBackdrop && event.target === dialog && !busy && emit('close')
      "
    >
      <div class="modal-content">
        <header class="modal-header">
          <div>
            <h2 :id="titleId">{{ title }}</h2>
            <p v-if="subtitle">{{ subtitle }}</p>
          </div>
          <button
            class="icon-button"
            type="button"
            aria-label="关闭弹窗"
            :disabled="busy"
            @click="emit('close')"
          >
            <Icon name="close" />
          </button>
        </header>
        <div class="modal-body"><slot /></div>
        <footer v-if="$slots.footer" class="modal-footer">
          <slot name="footer" />
        </footer>
      </div>
    </dialog>
  </Teleport>
</template>
