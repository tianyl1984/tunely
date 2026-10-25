<script setup>
import { computed, onBeforeUnmount, onMounted, reactive, ref } from "vue";
import { api, desktop, errorMessage } from "./services/api";
import Icon from "./components/Icon.vue";
import ModalShell from "./components/ModalShell.vue";
import ConnectionEditor from "./components/ConnectionEditor.vue";
import "./style.css";

const connections = ref([]);
const statuses = ref({});
const configPath = ref("");
const selected = ref(null);
const query = ref("");
const filter = ref("all");
const loading = ref(true);
const loadError = ref("");
const editor = ref(null);
const deleting = ref(null);
const deletingBusy = ref(false);
const trustPrompt = ref(null);
const trustBusy = ref(false);
const trustError = ref("");
const toast = ref(null);
const busyRules = reactive({});
const openingRules = reactive({});
const busyGroups = reactive({});
const testingId = ref(null);
const stoppingAll = ref(false);
let poll;
let toastTimer;
let polling = false;
let mounted = true;

const rules = computed(() => connections.value.flatMap((c) => c.rules));
const hasActive = computed(() =>
  Object.values(statuses.value).some((s) =>
    ["running", "starting"].includes(s.state),
  ),
);
const currentConnection = computed(() =>
  connections.value.find((c) => c.id === selected.value),
);

function stateOf(rule) {
  const status = statuses.value[rule.id] || {
    state: "stopped",
    activeStreams: 0,
  };
  return busyRules[rule.id] ? { ...status, state: busyRules[rule.id] } : status;
}
function isActive(rule) {
  return ["running", "starting", "stopping"].includes(stateOf(rule).state);
}
function connectionActive(connection) {
  return connection.rules.some(isActive);
}
function activeCount(connection) {
  return connection.rules.filter((r) => stateOf(r).state === "running").length;
}
function stateLabel(rule) {
  return (
    {
      running: "已开启",
      starting: "连接中",
      stopping: "停止中",
      stopped: "已停止",
      error: "连接异常",
    }[stateOf(rule).state] || "已停止"
  );
}
function localAddress(rule) {
  return `${rule.bindHost === "127.0.0.1" ? "localhost" : rule.bindHost}:${rule.localPort}`;
}
function remoteAddress(rule) {
  return `${rule.remoteHost.includes(":") ? `[${rule.remoteHost}]` : rule.remoteHost}:${rule.remotePort}`;
}

const visibleConnections = computed(() => {
  const search = query.value.trim().toLowerCase();
  return connections.value
    .filter((c) => !selected.value || c.id === selected.value)
    .map((c) => {
      const matchesConnection = `${c.name} ${c.host} ${c.username}`
        .toLowerCase()
        .includes(search);
      const visibleRules = c.rules.filter((r) => {
        const matchesSearch =
          matchesConnection ||
          `${r.name} ${localAddress(r)} ${remoteAddress(r)}`
            .toLowerCase()
            .includes(search);
        const active = isActive(r);
        return (
          matchesSearch &&
          (filter.value === "all" ||
            (filter.value === "running" ? active : !active))
        );
      });
      return { ...c, visibleRules, matchesConnection };
    })
    .filter(
      (c) =>
        c.visibleRules.length ||
        (!c.rules.length && filter.value !== "running" && c.matchesConnection),
    );
});

function notify(message, type = "success") {
  toast.value = { message, type };
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => {
    toast.value = null;
  }, 6500);
}

async function refreshStatuses() {
  if (polling || !mounted) return;
  polling = true;
  try {
    const values = await api.statuses();
    if (mounted)
      statuses.value = Object.fromEntries(
        values.map((status) => [status.ruleId, status]),
      );
  } catch (cause) {
    // Retain the last known state and surface IPC errors instead of showing all stopped.
    if (!toast.value)
      notify(`无法更新转发状态：${errorMessage(cause)}`, "error");
  } finally {
    polling = false;
  }
}

async function load() {
  loading.value = true;
  loadError.value = "";
  try {
    const data = await api.load();
    connections.value = data.connections;
    configPath.value = data.configPath;
    await refreshStatuses();
  } catch (cause) {
    loadError.value = errorMessage(cause);
  } finally {
    loading.value = false;
  }
}

function runTrusted(operation) {
  return operation().catch((cause) => {
    if (!cause?.hostKey) throw cause;
    if (trustPrompt.value) throw new Error("请先处理当前的服务器指纹确认");
    return new Promise((resolve, reject) => {
      trustError.value = "";
      trustPrompt.value = {
        challenge: cause.hostKey,
        operation,
        resolve,
        reject,
      };
    });
  });
}

function cancelTrust() {
  const prompt = trustPrompt.value;
  trustPrompt.value = null;
  prompt?.reject(new Error("已取消连接，服务器指纹未被信任"));
}

async function confirmTrust() {
  const prompt = trustPrompt.value;
  trustBusy.value = true;
  try {
    await api.trust(prompt.challenge);
    trustPrompt.value = null;
    trustBusy.value = false;
    runTrusted(prompt.operation).then(prompt.resolve, prompt.reject);
  } catch (cause) {
    if (trustPrompt.value === prompt) trustError.value = errorMessage(cause);
    else prompt.reject(cause);
  } finally {
    trustBusy.value = false;
  }
}

function saved(connection) {
  const index = connections.value.findIndex((c) => c.id === connection.id);
  if (index >= 0) connections.value[index] = connection;
  else connections.value.push(connection);
  editor.value = null;
  notify("连接配置已保存");
}

async function removeConnection() {
  const connection = deleting.value;
  deletingBusy.value = true;
  try {
    await api.remove(connection.id);
    connections.value = connections.value.filter((c) => c.id !== connection.id);
    if (selected.value === connection.id) selected.value = null;
    deleting.value = null;
    await refreshStatuses();
    notify("连接及转发规则已删除");
  } catch (cause) {
    notify(errorMessage(cause), "error");
  } finally {
    deletingBusy.value = false;
  }
}

async function testSaved(connection) {
  testingId.value = connection.id;
  try {
    const result = await runTrusted(() =>
      api.test({
        connection: JSON.parse(JSON.stringify(connection)),
        password: null,
        passphrase: null,
        clearPassphrase: false,
      }),
    );
    notify(`${connection.name} 连接成功 · ${result.latencyMs} ms`);
  } catch (cause) {
    notify(errorMessage(cause), "error");
  } finally {
    testingId.value = null;
  }
}

async function startRule(connection, rule) {
  busyRules[rule.id] = "starting";
  try {
    await runTrusted(() => api.start(connection.id, rule.id));
    return true;
  } catch (cause) {
    if (busyRules[rule.id] === "starting") notify(errorMessage(cause), "error");
    return false;
  } finally {
    if (busyRules[rule.id] === "starting") delete busyRules[rule.id];
    await refreshStatuses();
  }
}

async function stopRule(rule) {
  busyRules[rule.id] = "stopping";
  try {
    await api.stop(rule.id);
  } catch (cause) {
    notify(errorMessage(cause), "error");
  } finally {
    delete busyRules[rule.id];
    await refreshStatuses();
  }
}

function toggle(connection, rule) {
  if (isActive(rule)) stopRule(rule);
  else startRule(connection, rule);
}

async function openPort(connection, rule) {
  openingRules[rule.id] = true;
  try {
    await api.openPort(connection.id, rule.id);
  } catch (cause) {
    notify(errorMessage(cause), "error");
  } finally {
    delete openingRules[rule.id];
  }
}

function toggleDisabled(rule) {
  if (stateOf(rule).state === "stopping") return true;
  // Cancellation becomes available once the native task has been reserved.
  return (
    busyRules[rule.id] === "starting" &&
    statuses.value[rule.id]?.state !== "starting"
  );
}

async function toggleGroup(connection) {
  busyGroups[connection.id] = true;
  try {
    if (connectionActive(connection)) await api.stopAll(connection.id);
    else {
      for (const rule of connection.rules) {
        if (!isActive(rule) && !(await startRule(connection, rule))) break;
      }
    }
  } catch (cause) {
    notify(errorMessage(cause), "error");
  } finally {
    delete busyGroups[connection.id];
    await refreshStatuses();
  }
}

async function stopAll() {
  stoppingAll.value = true;
  try {
    await api.stopAll();
    notify("所有转发已停止");
  } catch (cause) {
    notify(errorMessage(cause), "error");
  } finally {
    stoppingAll.value = false;
    await refreshStatuses();
  }
}

function selectConnection(id) {
  selected.value = id;
  query.value = "";
  filter.value = "all";
}
onMounted(async () => {
  await load();
  poll = setInterval(refreshStatuses, 1000);
});
onBeforeUnmount(() => {
  mounted = false;
  clearInterval(poll);
  clearTimeout(toastTimer);
  cancelTrust();
});
</script>

<template>
  <div class="app-shell">
    <aside class="sidebar">
      <div class="brand">
        <img class="brand-symbol" src="/tunely.svg" alt="Tunely" width="36" height="36" />
        <div><strong>Tunely</strong><span>SSH PORT FORWARDING</span></div>
      </div>
      <p class="nav-caption">工作空间</p>
      <button
        class="nav-item"
        :class="{ active: !selected }"
        @click="selectConnection(null)"
      >
        <Icon name="grid" /> 所有转发
        <span class="nav-count">{{ rules.length }}</span>
      </button>
      <div class="nav-caption connection-caption">
        <span>我的连接</span
        ><button class="icon-button" aria-label="新建连接" @click="editor = {}">
          <Icon name="plus" :size="15" />
        </button>
      </div>
      <nav class="connection-nav" aria-label="SSH 连接列表">
        <button
          v-for="connection in connections"
          :key="connection.id"
          class="connection-nav-item"
          :class="{ active: selected === connection.id }"
          @click="selectConnection(connection.id)"
        >
          <span
            class="connection-dot"
            :class="{ online: connectionActive(connection) }"
          /><span class="truncate">{{ connection.name }}</span
          ><span class="nav-count">{{ connection.rules.length }}</span>
        </button>
        <p v-if="!connections.length && !loading" class="sidebar-empty">
          添加你的第一台服务器<br />让本地与远程连接起来
        </p>
      </nav>
    </aside>

    <main class="main-panel">
      <div class="page-content">
        <div class="page-heading">
          <div>
            <p class="eyebrow">YOUR LOCAL CONNECTIONS</p>
            <h1>{{ currentConnection?.name || "端口转发" }}</h1>
            <p class="page-description">
              {{
                currentConnection
                  ? `${currentConnection.username}@${currentConnection.host}:${currentConnection.port}`
                  : "一处管理 SSH 连接，随时开启你的远程服务。"
              }}
            </p>
          </div>
          <button
            class="button primary"
            :disabled="loading || !!loadError"
            @click="editor = {}"
          >
            <Icon name="plus" :size="17" /> 新建连接
          </button>
        </div>

        <div v-if="!desktop" class="preview-banner">
          <Icon name="alert" :size="15" />
          浏览器预览仅保存配置；连接测试与真实转发请在桌面应用中使用。
        </div>
        <div v-if="loadError" class="load-error notice error" role="alert">
          <Icon name="alert" /><span>{{ loadError }}</span
          ><button class="button secondary" @click="load">重试</button>
        </div>
        <div v-else-if="loading" class="empty-state">
          <span class="spinner" />
          <p>正在读取本地配置…</p>
        </div>
        <template v-else>
          <div v-if="connections.length" class="list-toolbar">
            <div class="filter-tabs" aria-label="转发状态筛选">
              <button
                v-for="item in [
                  { value: 'all', label: '全部规则' },
                  { value: 'running', label: '已开启' },
                  { value: 'stopped', label: '未开启' },
                ]"
                :key="item.value"
                :class="{ active: filter === item.value }"
                :aria-pressed="filter === item.value"
                @click="filter = item.value"
              >
                {{ item.label }}
              </button>
            </div>
            <div class="toolbar-actions">
              <button
                v-if="hasActive"
                class="text-button danger-text"
                :disabled="stoppingAll"
                @click="stopAll"
              >
                <Icon name="stop" :size="14" /> 全部停止</button
              ><label class="search-field"
                ><Icon name="search" :size="16" /><input
                  v-model="query"
                  placeholder="搜索连接、端口…"
                  aria-label="搜索连接或端口"
              /></label>
            </div>
          </div>

          <div v-if="!connections.length" class="empty-state first-connection">
            <div class="empty-diagram">
              <span><Icon name="server" :size="27" /></span><i /><span
                class="center-link"
                ><Icon name="link" :size="23" /></span
              ><i /><span><Icon name="server" :size="27" /></span>
            </div>
            <h2>你的远程服务，近在本地</h2>
            <p>
              添加 SSH 连接与端口规则，<br />将远程服务器上的服务带到
              localhost。
            </p>
            <button class="button primary" @click="editor = {}">
              <Icon name="plus" :size="17" /> 添加第一个连接
            </button>
            <div class="empty-example">
              <code>localhost:8000</code><Icon name="arrow" :size="16" /><code
                >远程服务器:8080</code
              >
            </div>
          </div>
          <div
            v-else-if="!visibleConnections.length"
            class="empty-state search-empty"
          >
            <Icon name="search" :size="30" />
            <h2>没有匹配的转发规则</h2>
            <p>试试其他关键词，或切换到全部规则。</p>
            <button
              class="button secondary"
              @click="
                query = '';
                filter = 'all';
              "
            >
              清除筛选
            </button>
          </div>

          <article
            v-for="connection in visibleConnections"
            :key="connection.id"
            class="connection-card"
          >
            <header class="connection-card-header">
              <span class="server-avatar"
                ><Icon name="server" :size="21"
              /></span>
              <div class="connection-info">
                <h2>
                  {{ connection.name
                  }}<span class="auth-badge"
                    ><Icon
                      :name="
                        connection.authType === 'password' ? 'lock' : 'key'
                      "
                      :size="11"
                    />{{
                      connection.authType === "password" ? "密码" : "私钥"
                    }}</span
                  >
                </h2>
                <p>
                  {{ connection.username }}@{{ connection.host
                  }}<span class="port-text">:{{ connection.port }}</span>
                </p>
              </div>
              <div class="card-actions">
                <button
                  class="icon-button"
                  :aria-label="`测试连接 ${connection.name}`"
                  title="测试连接"
                  :disabled="!!testingId"
                  @click="testSaved(connection)"
                >
                  <span
                    v-if="testingId === connection.id"
                    class="spinner"
                  /><Icon v-else name="activity" :size="17" /></button
                ><button
                  class="icon-button"
                  :aria-label="`编辑连接 ${connection.name}`"
                  :title="
                    connectionActive(connection)
                      ? '请先停止此连接下的转发'
                      : '编辑连接'
                  "
                  :disabled="
                    connectionActive(connection) || !!busyGroups[connection.id]
                  "
                  @click="
                    editor = {
                      connection: connections.find(
                        (item) => item.id === connection.id,
                      ),
                    }
                  "
                >
                  <Icon name="edit" :size="17" /></button
                ><button
                  class="icon-button delete-connection"
                  :aria-label="`删除连接 ${connection.name}`"
                  title="删除连接"
                  :disabled="!!busyGroups[connection.id]"
                  @click="deleting = connection"
                >
                  <Icon name="trash" :size="17" /></button
                ><span class="action-divider" /><button
                  class="button small"
                  :class="
                    connectionActive(connection) ? 'secondary' : 'soft-primary'
                  "
                  :disabled="
                    !connection.rules.length || !!busyGroups[connection.id]
                  "
                  @click="toggleGroup(connection)"
                >
                  <span v-if="busyGroups[connection.id]" class="spinner" /><Icon
                    v-else
                    :name="connectionActive(connection) ? 'stop' : 'play'"
                    :size="13"
                  />
                  {{ connectionActive(connection) ? "停止全部" : "开启全部" }}
                </button>
              </div>
            </header>
            <div v-if="connection.visibleRules.length" class="forward-table">
              <div class="table-heading">
                <span>转发规则 / 本地端口</span><span /><span>远程目标</span
                ><span>状态</span><span class="align-right">操作</span>
              </div>
              <div
                v-for="(rule, index) in connection.visibleRules"
                :key="rule.id"
                class="forward-row"
              >
                <div class="local-cell">
                  <span class="rule-name">{{
                    rule.name || `端口转发 ${index + 1}`
                  }}</span
                  ><code>{{ localAddress(rule) }}</code
                  ><span v-if="rule.bindHost === '0.0.0.0'" class="bind-caption"
                    >所有网卡</span
                  >
                </div>
                <span class="direction-icon"
                  ><Icon name="arrow" :size="18"
                /></span>
                <div class="remote-cell">
                  <code>{{ remoteAddress(rule) }}</code
                  ><span>经 SSH 加密转发</span>
                </div>
                <div class="status-cell">
                  <span class="status-badge" :class="stateOf(rule).state"
                    ><span
                      v-if="
                        ['starting', 'stopping'].includes(stateOf(rule).state)
                      "
                      class="spinner"
                    /><span v-else class="status-dot" />{{
                      stateLabel(rule)
                    }}</span
                  ><span
                    v-if="stateOf(rule).activeStreams > 0"
                    class="stream-count"
                    >{{ stateOf(rule).activeStreams }} 个数据连接</span
                  >
                </div>
                <div class="rule-actions">
                  <button
                    class="toggle"
                    :class="{
                      on: isActive(rule),
                      pending: ['starting', 'stopping'].includes(
                        stateOf(rule).state,
                      ),
                    }"
                    role="switch"
                    :aria-checked="isActive(rule)"
                    :aria-label="`${isActive(rule) ? '停止' : '开启'}转发 ${rule.name || localAddress(rule)}`"
                    :disabled="
                      toggleDisabled(rule) || !!busyGroups[connection.id]
                    "
                    @click="toggle(connection, rule)"
                  >
                    <span />
                  </button>
                  <button
                    class="text-button open-port"
                    :aria-label="`打开本地端口 ${rule.localPort}`"
                    :title="`使用默认浏览器打开 http://127.0.0.1:${rule.localPort}`"
                    :disabled="!!openingRules[rule.id]"
                    @click="openPort(connection, rule)"
                  >
                    <span v-if="openingRules[rule.id]" class="spinner" />
                    <Icon v-else name="external" :size="13" />
                    打开
                  </button>
                </div>
                <div v-if="stateOf(rule).lastError" class="rule-error">
                  <Icon name="alert" :size="13" /><span>{{
                    stateOf(rule).lastError
                  }}</span>
                </div>
              </div>
            </div>
            <div v-else class="card-empty">
              尚未添加转发规则，点击编辑开始配置。
            </div>
            <footer class="connection-card-footer">
              <span
                ><span
                  class="connection-dot"
                  :class="{ online: activeCount(connection) > 0 }"
                />{{ activeCount(connection) }} /
                {{ connection.rules.length }} 条规则已开启</span
              ><span>{{
                connectionActive(connection)
                  ? "停止转发后可编辑配置"
                  : "配置已保存到本地"
              }}</span>
            </footer>
          </article>
        </template>
      </div>
      <footer class="page-footer">
        <span
          ><Icon name="shield" :size="13" />
          {{
            desktop
              ? "配置仅存本机 · SSH 加密传输"
              : "预览配置与桌面配置独立存储"
          }}</span
        ><span v-if="configPath" class="storage-path" :title="configPath">{{
          desktop ? configPath : "Tunely"
        }}</span>
      </footer>
    </main>

    <ConnectionEditor
      v-if="editor"
      :connection="editor.connection"
      :run-trusted="runTrusted"
      @close="editor = null"
      @saved="saved"
    />
    <ModalShell
      v-if="deleting"
      title="删除这个连接？"
      :busy="deletingBusy"
      @close="deleting = null"
      ><p class="dialog-copy">
        将删除 <strong>{{ deleting.name }}</strong> 及其
        {{ deleting.rules.length }}
        条转发规则。正在运行的转发会停止，已保存的凭据也会移除。
      </p>
      <template #footer
        ><button
          class="button secondary"
          :disabled="deletingBusy"
          @click="deleting = null"
        >
          取消</button
        ><button
          class="button danger"
          :disabled="deletingBusy"
          @click="removeConnection"
        >
          <span v-if="deletingBusy" class="spinner" /> 确认删除
        </button></template
      ></ModalShell
    >
    <ModalShell
      v-if="trustPrompt"
      :title="
        trustPrompt.challenge.previousFingerprint
          ? '服务器指纹已变化'
          : '确认服务器身份'
      "
      subtitle="确认指纹与服务器提供的指纹一致后再继续。"
      :busy="trustBusy"
      @close="cancelTrust"
      ><div class="host-identity">
        <Icon name="shield" :size="25" /><strong
          >{{ trustPrompt.challenge.host }}:{{
            trustPrompt.challenge.port
          }}</strong
        >
      </div>
      <p class="dialog-copy">
        {{
          trustPrompt.challenge.previousFingerprint
            ? "该服务器的 SSH 密钥与之前保存的记录不同，请先核实服务器是否更换了密钥。"
            : "这是首次连接此服务器，信任后将把指纹保存到本地，后续连接会自动核验。"
        }}
      </p>
      <label class="fingerprint-label"
        >当前 SHA256 指纹<code class="fingerprint">{{
          trustPrompt.challenge.fingerprint
        }}</code></label
      ><label
        v-if="trustPrompt.challenge.previousFingerprint"
        class="fingerprint-label"
        >此前保存的指纹<code class="fingerprint previous">{{
          trustPrompt.challenge.previousFingerprint
        }}</code></label
      >
      <p v-if="trustError" class="notice error" role="alert">
        {{ trustError }}
      </p>
      <template #footer
        ><button
          class="button secondary"
          :disabled="trustBusy"
          @click="cancelTrust"
        >
          取消连接</button
        ><button
          class="button"
          :class="
            trustPrompt.challenge.previousFingerprint ? 'danger' : 'primary'
          "
          :disabled="trustBusy"
          @click="confirmTrust"
        >
          <span v-if="trustBusy" class="spinner" /> 信任并继续
        </button></template
      ></ModalShell
    >
    <div v-if="toast" class="toast" :class="toast.type" role="status">
      <Icon
        :name="toast.type === 'error' ? 'alert' : 'check'"
        :size="18"
      /><span>{{ toast.message }}</span
      ><button class="icon-button" aria-label="关闭提示" @click="toast = null">
        <Icon name="close" :size="14" />
      </button>
    </div>
  </div>
</template>
