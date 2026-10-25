<script setup>
import { reactive, ref } from "vue";
import {
  api,
  desktop,
  errorMessage,
  newConnection,
  newRule,
  validateInput,
} from "../services/api";
import Icon from "./Icon.vue";
import ModalShell from "./ModalShell.vue";

const props = defineProps({ connection: Object, runTrusted: Function });
const emit = defineEmits(["close", "saved"]);
const draft = reactive(
  JSON.parse(JSON.stringify(props.connection || newConnection())),
);
const password = ref("");
const passphrase = ref("");
const clearPassphrase = ref(false);
const error = ref("");
const testing = ref(false);
const saving = ref(false);
const testResult = ref(null);

function input() {
  const connection = JSON.parse(JSON.stringify(draft));
  connection.port = Number(connection.port);
  connection.rules = connection.rules.map((rule) => ({
    ...rule,
    localPort: Number(rule.localPort),
    remotePort: Number(rule.remotePort),
  }));
  return {
    connection,
    password: password.value || null,
    passphrase: passphrase.value || null,
    clearPassphrase: clearPassphrase.value,
  };
}

async function test() {
  const payload = input();
  error.value = validateInput(payload, false);
  if (error.value) return;
  testResult.value = null;
  testing.value = true;
  try {
    testResult.value = await props.runTrusted(() => api.test(payload));
  } catch (cause) {
    error.value = errorMessage(cause);
  } finally {
    testing.value = false;
  }
}

async function save() {
  const payload = input();
  error.value = validateInput(payload);
  if (error.value) return;
  saving.value = true;
  try {
    emit("saved", await api.save(payload));
  } catch (cause) {
    error.value = errorMessage(cause);
  } finally {
    saving.value = false;
  }
}

async function chooseKey() {
  try {
    const path = await api.chooseKey();
    if (path) {
      draft.privateKeyPath = path;
      passphrase.value = "";
      clearPassphrase.value = true;
    }
  } catch (cause) {
    error.value = errorMessage(cause);
  }
}

function addRule() {
  draft.rules.push(newRule());
}
</script>

<template>
  <ModalShell
    :title="connection ? '编辑连接' : '新建 SSH 连接'"
    subtitle="连接远程服务器，配置你的本地端口转发。"
    wide
    :close-on-backdrop="false"
    :busy="saving || testing"
    @close="emit('close')"
  >
    <form
      id="connection-editor"
      @submit.prevent="save"
      @input="testResult = null"
    >
      <section class="form-section">
        <h3><span class="section-number">01</span> 连接信息</h3>
        <div class="form-grid">
          <label class="field full"
            >连接名称
            <input
              v-model="draft.name"
              name="connectionName"
              placeholder="例如：开发服务器"
              maxlength="80"
              required
              autofocus
          /></label>
          <label class="field"
            >SSH 主机
            <input
              v-model="draft.host"
              name="host"
              placeholder="IP 地址或域名"
              required
          /></label>
          <label class="field"
            >SSH 端口
            <input
              v-model="draft.port"
              name="sshPort"
              type="number"
              min="1"
              max="65535"
              required
          /></label>
          <label class="field full"
            >用户名
            <input
              v-model="draft.username"
              name="username"
              autocomplete="off"
              placeholder="root"
              required
          /></label>
        </div>
      </section>

      <section class="form-section">
        <h3><span class="section-number">02</span> 身份认证</h3>
        <div class="auth-picker" role="group" aria-label="认证方式">
          <label :class="{ selected: draft.authType === 'password' }"
            ><input
              v-model="draft.authType"
              type="radio"
              value="password"
            /><Icon name="lock" :size="16" /> 密码认证</label
          >
          <label :class="{ selected: draft.authType === 'privateKey' }"
            ><input
              v-model="draft.authType"
              type="radio"
              value="privateKey"
            /><Icon name="key" :size="16" /> 私钥认证</label
          >
        </div>
        <label v-if="draft.authType === 'password'" class="field"
          >登录密码
          <input
            v-model="password"
            name="password"
            type="password"
            autocomplete="new-password"
            :placeholder="
              draft.hasPassword
                ? '已保存密码，留空保持不变'
                : '输入 SSH 登录密码'
            "
            :required="!draft.hasPassword"
          />
        </label>
        <template v-else>
          <label class="field"
            >私钥文件
            <span class="file-field"
              ><input
                v-model="draft.privateKeyPath"
                name="privateKeyPath"
                placeholder="~/.ssh/id_ed25519"
                required
              /><button
                class="button secondary"
                type="button"
                @click="chooseKey"
              >
                <Icon name="folder" :size="16" /> 选择文件
              </button></span
            >
          </label>
          <label class="field spaced"
            >私钥口令 <span class="optional">可选</span
            ><input
              v-model="passphrase"
              name="passphrase"
              type="password"
              autocomplete="new-password"
              :placeholder="
                draft.hasPassphrase && !clearPassphrase
                  ? '已保存口令，留空保持不变'
                  : '私钥未加密时留空'
              "
          /></label>
          <label v-if="draft.hasPassphrase" class="checkbox-field"
            ><input v-model="clearPassphrase" type="checkbox" />
            清除已保存的私钥口令</label
          >
        </template>
        <p class="field-hint">
          <Icon name="shield" :size="13" />
          {{
            desktop
              ? "密码与私钥口令保存到系统凭据库，私钥文件保留在原位置。"
              : "浏览器预览仅保存配置，不保存密码与口令；SSH 操作需在桌面应用中执行。"
          }}
        </p>
      </section>

      <section class="form-section rules-section">
        <div class="section-heading">
          <h3>
            <span class="section-number">03</span> 端口转发
            <span class="muted">{{ draft.rules.length }}</span>
          </h3>
          <button class="text-button" type="button" @click="addRule">
            <Icon name="plus" :size="15" /> 添加规则
          </button>
        </div>
        <p class="section-description">
          访问本地端口，流量经 SSH 转发至远程网络中的目标地址。
        </p>
        <div v-if="!draft.rules.length" class="rules-empty">
          暂无转发规则，保存后也可以随时添加。
        </div>
        <fieldset
          v-for="(rule, index) in draft.rules"
          :key="rule.id"
          class="rule-editor"
        >
          <legend>规则 {{ String(index + 1).padStart(2, "0") }}</legend>
          <div class="rule-heading">
            <label class="field grow"
              >规则名称 <span class="optional">可选</span
              ><input
                v-model="rule.name"
                :name="`ruleName-${index}`"
                placeholder="例如：Web 服务 / 数据库"
                maxlength="80" /></label
            ><button
              class="icon-button danger-text"
              type="button"
              :aria-label="`删除转发规则 ${index + 1}`"
              @click="draft.rules.splice(index, 1)"
            >
              <Icon name="trash" :size="16" />
            </button>
          </div>
          <div class="rule-fields">
            <label class="field"
              >本地监听
              <select v-model="rule.bindHost" :name="`bindHost-${index}`">
                <option value="127.0.0.1">localhost · 仅本机</option>
                <option value="0.0.0.0">0.0.0.0 · 所有网卡</option>
              </select></label
            >
            <label class="field"
              >本地端口
              <input
                v-model="rule.localPort"
                :name="`localPort-${index}`"
                type="number"
                placeholder="如 8000"
                min="1"
                max="65535"
                required
            /></label>
            <span class="rule-arrow"><Icon name="arrow" :size="18" /></span>
            <label class="field"
              >目标主机
              <input
                v-model="rule.remoteHost"
                :name="`remoteHost-${index}`"
                placeholder="127.0.0.1"
                required
            /></label>
            <label class="field"
              >远程端口
              <input
                v-model="rule.remotePort"
                :name="`remotePort-${index}`"
                type="number"
                placeholder="如 8080"
                min="1"
                max="65535"
                required
            /></label>
          </div>
          <p v-if="rule.bindHost === '0.0.0.0'" class="field-hint exposure">
            <Icon name="alert" :size="13" /> 所有网卡监听允许其他设备通过本机 IP
            访问此端口。
          </p>
        </fieldset>
      </section>
      <div v-if="error" class="notice error" role="alert">
        <Icon name="alert" :size="17" /><span>{{ error }}</span>
      </div>
      <div v-if="testResult" class="notice success" role="status">
        <Icon name="check" :size="17" />
        <div>
          <strong>{{ testResult.message }}</strong
          ><span class="test-details"
            >{{ testResult.latencyMs }} ms · {{ testResult.fingerprint }}</span
          >
        </div>
      </div>
    </form>
    <template #footer>
      <button
        class="button secondary"
        type="button"
        :disabled="testing || saving"
        @click="test"
      >
        <span v-if="testing" class="spinner" /><Icon
          v-else
          name="activity"
          :size="16"
        />
        {{ testing ? "测试连接中…" : "测试连接" }}
      </button>
      <span class="footer-spacer" />
      <button
        class="button primary"
        type="submit"
        form="connection-editor"
        :disabled="saving || testing"
      >
        <span v-if="saving" class="spinner" /><Icon
          v-else
          name="check"
          :size="16"
        />
        {{ saving ? "保存中…" : "保存连接" }}
      </button>
    </template>
  </ModalShell>
</template>
