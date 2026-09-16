const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");
const source = fs.readFileSync(
  require("node:path").join(__dirname, "../src/agent/pair.js"),
  "utf8",
);
function fixture(initial) {
  const elements = new Map();
  function element(id) {
    if (!elements.has(id)) {
      const classes = new Set();
      elements.set(id, {
        hidden: false,
        textContent: "",
        value: "",
        dataset: {},
        listeners: {},
        replaceChildren() {},
        classList: {
          toggle(c, on) {
            on ? classes.add(c) : classes.delete(c);
          },
          add(c) {
            classes.add(c);
          },
          remove(c) {
            classes.delete(c);
          },
          contains(c) {
            return classes.has(c);
          },
        },
        addEventListener(type, handler) {
          this.listeners[type] = handler;
        },
        showModal() {
          this.open = true;
        },
      });
    }
    return elements.get(id);
  }
  const buttons = ["start", "stop", "restart"].map((action) => ({
    dataset: { action },
    disabled: true,
  }));
  const calls = [];
  const state = { data: initial, offline: false, reject: false };
  const context = vm.createContext({
    URL,
    document: {
      getElementById: element,
      querySelectorAll: () => buttons,
      querySelector: () => ({ content: "csrf-token" }),
    },
    location: {
      pathname: "/",
      assign(url) {
        calls.push({ redirect: url });
      },
    },
    history: { replaceState() {} },
    setInterval() {},
    async fetch(url, options) {
      if (url.endsWith("/status")) {
        if (state.offline) throw Error("offline");
        return { ok: true, json: async () => state.data };
      }
      calls.push({ url, options });
      return {
        ok: !state.reject,
        json: async () =>
          state.reject
            ? { error: "rejected" }
            : {
                user_code: "ABCD",
                verification_uri: "https://cloud.camofy.app/authorize",
              },
      };
    },
  });
  vm.runInContext(source, context);
  return {
    element,
    buttons,
    calls,
    state,
    context,
    flush: () => new Promise((resolve) => setImmediate(resolve)),
    poll: () => vm.runInContext("status()", context),
  };
}
const bound = () => ({
  bound: true,
  authorized: true,
  cloud_url: "https://camofy.app",
  identity_name: "<home>",
  agent_version: "0.1.3",
  runtime: { core_state: "stopped", revision: "revision" },
});
test("bound dashboard is read-only for identity; public console and self-host links", async () => {
  const f = fixture(bound());
  await f.flush();
  assert.equal(f.element("binding").hidden, true);
  assert.equal(f.element("dashboard").hidden, false);
  assert.equal(f.element("identity-name").textContent, "<home>");
  assert.equal(f.element("manage").href, "https://cloud.camofy.app/");
  assert.equal(f.element("core-state").textContent, "已停止");
  assert.equal(f.element("version").textContent, "v0.1.3");
  f.state.data.cloud_url = "https://self.example";
  await f.poll();
  assert.equal(f.element("manage").href, "https://self.example/");
});
test("control requires confirmation; cancellation and Escape are safe; CSRF and errors retained", async () => {
  const f = fixture(bound());
  await f.flush();
  f.element("controls").listeners.click({
    target: { closest: () => f.buttons[0] },
  });
  assert.equal(f.element("confirm-dialog").open, true);
  assert.match(f.element("confirm-description").textContent, /TUN/);
  await f.element("confirm-dialog").listeners.close();
  assert.equal(f.calls.length, 0);
  f.element("confirm-dialog").returnValue = "confirm";
  await f.element("confirm-dialog").listeners.close();
  await f.flush();
  assert.equal(JSON.parse(f.calls[0].options.body).action, "start");
  assert.equal(f.calls[0].options.headers["X-Camofy-CSRF"], "csrf-token");
  f.element("controls").listeners.click({
    target: { closest: () => f.buttons[2] },
  });
  assert.equal(f.element("confirm-dialog").returnValue, "cancel");
  await f.element("confirm-dialog").listeners.close();
  assert.equal(f.calls.length, 1);
  f.state.reject = true;
  f.element("confirm-dialog").returnValue = "confirm";
  await f.element("confirm-dialog").listeners.close();
  await f.flush();
  assert.equal(f.element("control-result").textContent, "rejected");
  assert.equal(f.element("control-result").classList.contains("error"), true);
});
test("offline disables controls; recovery restores status without claiming cloud connectivity", async () => {
  const f = fixture(bound());
  await f.flush();
  f.state.offline = true;
  await f.poll();
  assert.ok(f.buttons.every((b) => b.disabled));
  assert.equal(f.element("core-state").textContent, "连接中断");
  f.state.offline = false;
  await f.poll();
  assert.ok(f.buttons.every((b) => !b.disabled));
  assert.equal(f.element("message").hidden, true);
});
test("expired binding can retry without terminal polling hiding the form", async () => {
  const f = fixture({ bound: false, phase: "expired" });
  await f.flush();
  assert.equal(f.element("retry").hidden, false);
  assert.equal(f.element("form").hidden, true);
  f.element("retry").listeners.click();
  await f.poll();
  assert.equal(f.element("form").hidden, false);
  f.element("name").value = "test router";
  f.element("cloud").value = "https://camofy.app/";
  await f.element("form").listeners.submit({ preventDefault() {} });
  assert.deepEqual(JSON.parse(f.calls[0].options.body), {
    device_name: "test router",
    cloud_url: "https://camofy.app/",
  });
  assert.equal(f.calls[1].redirect, "https://cloud.camofy.app/authorize");
});
test("bound LAN visitors cannot control the device without unlocking",async()=>{
  const f=fixture({...bound(),authorized:false});await f.flush();
  assert.equal(f.element("unlock").hidden,false);
  assert.equal(f.element("dashboard").hidden,true);
  assert.equal(f.element("proxy-panel").hidden,true);
  assert.ok(f.buttons.every(b=>b.disabled));
});
