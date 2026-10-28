import { invoke, isTauri } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

export const desktop = isTauri();
const PREVIEW_KEY = "tunely.preview.connections.v1";

function previewConnections() {
  const value = JSON.parse(localStorage.getItem(PREVIEW_KEY) || "[]");
  if (!Array.isArray(value)) throw new Error("浏览器预览配置格式错误");
  return value;
}

export function errorMessage(error) {
  return (
    error?.message || (typeof error === "string" ? error : "操作失败，请重试")
  );
}

function nativeOnly() {
  throw new Error("请通过 ./dev.sh 启动桌面应用后执行 SSH 操作。");
}

export const api = {
  async load() {
    if (desktop) return invoke("load_connections");
    return {
      connections: previewConnections(),
      configPath: "浏览器预览：仅保存配置，不保存密码或私钥口令",
    };
  },
  async save(input) {
    if (desktop) return invoke("save_connection", { input });
    // Browser preview persists metadata only. It never opens SSH connections.
    const connection = {
      ...input.connection,
      hasPassword: false,
      hasPassphrase: false,
    };
    const connections = previewConnections();
    const index = connections.findIndex((item) => item.id === connection.id);
    if (index >= 0) connections[index] = connection;
    else connections.push(connection);
    localStorage.setItem(PREVIEW_KEY, JSON.stringify(connections));
    return connection;
  },
  async remove(connectionId) {
    if (desktop) return invoke("delete_connection", { connectionId });
    localStorage.setItem(
      PREVIEW_KEY,
      JSON.stringify(
        previewConnections().filter((item) => item.id !== connectionId),
      ),
    );
  },
  async statuses() {
    return desktop ? invoke("tunnel_statuses") : [];
  },
  async openPort(connectionId, ruleId) {
    if (!desktop) throw new Error("请在桌面应用中使用默认浏览器打开本地端口。");
    return invoke("open_local_port", { connectionId, ruleId });
  },
  async test(input) {
    return desktop ? invoke("test_connection", { input }) : nativeOnly();
  },
  async start(connectionId, ruleId) {
    return desktop
      ? invoke("start_forward", { connectionId, ruleId })
      : nativeOnly();
  },
  async reconnect(ruleId) {
    return desktop ? invoke("reconnect_forward", { ruleId }) : nativeOnly();
  },
  async stop(ruleId) {
    return desktop ? invoke("stop_forward", { ruleId }) : nativeOnly();
  },
  async stopAll(connectionId = null) {
    return desktop
      ? invoke("stop_all_forwards", { connectionId })
      : nativeOnly();
  },
  async trust(challenge) {
    return desktop ? invoke("trust_host_key", { challenge }) : nativeOnly();
  },
  async chooseKey() {
    if (!desktop)
      throw new Error("请在桌面应用中选择私钥文件，浏览器预览可手动输入路径。");
    return open({
      title: "选择 SSH 私钥文件",
      multiple: false,
      directory: false,
    });
  },
};

export function newRule() {
  return {
    id: crypto.randomUUID(),
    name: "",
    bindHost: "127.0.0.1",
    localPort: "",
    remoteHost: "127.0.0.1",
    remotePort: "",
  };
}

export function newConnection() {
  return {
    id: crypto.randomUUID(),
    name: "",
    host: "",
    port: 22,
    username: "root",
    authType: "password",
    privateKeyPath: "",
    hasPassword: false,
    hasPassphrase: false,
    rules: [newRule()],
  };
}

export function validateInput(input, includeRules = true) {
  const c = input.connection;
  const validPort = (port) =>
    Number.isInteger(Number(port)) && Number(port) > 0 && Number(port) <= 65535;
  if (!c.name.trim()) return "请填写连接名称";
  if (!c.host.trim() || /\s/.test(c.host.trim()))
    return "请填写有效的 SSH 主机地址";
  if (!validPort(c.port)) return "SSH 端口必须在 1–65535 之间";
  if (!c.username.trim()) return "请填写 SSH 用户名";
  if (c.authType === "password" && !input.password && !c.hasPassword)
    return "请输入 SSH 登录密码";
  if (c.authType === "privateKey" && !c.privateKeyPath.trim())
    return "请选择私钥文件或输入文件路径";
  if (includeRules) {
    const ports = new Set();
    for (const [index, rule] of c.rules.entries()) {
      if (!validPort(rule.localPort) || !validPort(rule.remotePort))
        return `规则 ${index + 1}：端口必须在 1–65535 之间`;
      if (!rule.remoteHost.trim() || /\s/.test(rule.remoteHost.trim()))
        return `规则 ${index + 1}：请填写有效的目标主机`;
      if (ports.has(Number(rule.localPort)))
        return "同一连接中的本地端口不能重复";
      ports.add(Number(rule.localPort));
    }
  }
  return "";
}
