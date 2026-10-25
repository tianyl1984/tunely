<script setup>
defineProps({ name: String, size: { type: Number, default: 18 } });
const paths = {
  plus: "M12 5v14M5 12h14",
  close: "m6 6 12 12M6 18 18 6",
  arrow: "M4 12h16m-5-5 5 5-5 5",
  external:
    "M15 3h6v6m0-6L10 14M9 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-4",
  server:
    "M5 3h14a2 2 0 0 1 2 2v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2Zm0 10h14a2 2 0 0 1 2 2v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4a2 2 0 0 1 2-2ZM7 7h.01M7 17h.01",
  grid: "M3 3h7v7H3zm11 0h7v7h-7zM3 14h7v7H3zm11 0h7v7h-7z",
  search: "M21 21l-5-5M18 10a8 8 0 1 1-16 0 8 8 0 0 1 16 0Z",
  key: "m15 7 2 2m-5 3 8-8m-6 6 3 3 3-3-3-3M12 15a5 5 0 1 1-10 0 5 5 0 0 1 10 0Z",
  lock: "M5 11h14v10H5zm3 0V7a4 4 0 0 1 8 0v4m-4 4v2",
  shield: "M12 3 3 7v5c0 5 9 9 9 9s9-4 9-9V7l-9-4Zm-4 9 3 3 5-6",
  play: "m8 5 11 7-11 7Z",
  stop: "M6 6h12v12H6z",
  edit: "m16 3 5 5-13 13H3v-5L16 3Zm-2 2 5 5",
  trash: "M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7m4-7v7",
  check: "m5 12 4 4L19 6",
  alert: "M12 3 2 21h20L12 3Zm0 6v5m0 3h.01",
  folder: "M3 7V4h7l3 3h8v13H3V7Z",
  activity: "M3 12h4l3-8 4 16 3-8h4",
  link: "m10 14 4-4m-6 7-1 1a4 4 0 0 1-6-6l5-5a4 4 0 0 1 6 0m0 0 1-1a4 4 0 0 1 6 6l-5 5a4 4 0 0 1-6 0",
  chevron: "m9 5 7 7-7 7",
  clock: "M12 7v5l3 2M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0Z",
};
</script>
<template>
  <svg
    :width="size"
    :height="size"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="1.7"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >
    <path :d="paths[name] || paths.link" />
  </svg>
</template>
